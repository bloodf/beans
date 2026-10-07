//! Explicit ORT CPU integration. No catalog, download-binaries, hub or paid fallback.
use super::{assets::{verified_bytes,BoundAsset,LocalBinding,ProtectedRuntime},*};
use crate::memory_service::types::EmbeddingProfile;
use ort::{session::{RunOptions,Session},value::{Tensor,TensorRef,TensorElementType,ValueType}};
use parking_lot::Mutex;
use std::{collections::BTreeSet,sync::Arc};
use tokenizers::{Tokenizer,PaddingParams,PaddingDirection,PaddingStrategy,TruncationParams,TruncationDirection,TruncationStrategy};
use tokio_util::sync::CancellationToken;

struct RuntimeState {approved:BoundAsset,_protected:ProtectedRuntime,committed:bool}
static RUNTIME:Mutex<Option<RuntimeState>>=Mutex::new(None);
pub struct LocalEmbedding {
    spec:EmbeddingSpec,
    tokenizer:Arc<Tokenizer>,
    session:Arc<tokio::sync::Mutex<Session>>,
    limits:EmbeddingLimits,
}
impl LocalEmbedding {
    /// Trusted Runner calls after exact setup consent. Files are verified before native load.
    /// Runtime is process-global: a new runtime binding requires an explicit process restart.
    pub async fn load(profile:&EmbeddingProfile,binding:LocalBinding,limits:EmbeddingLimits,cancel:CancellationToken)->Result<Self,EmbeddingError> {
        let spec=EmbeddingSpec::from_profile(profile)?;limits.validate()?;
        if spec.mode!=EmbeddingMode::LocalCpu {return Err(EmbeddingError::InvalidProfile);}
        let config=spec.local.clone().ok_or(EmbeddingError::InvalidProfile)?;
        if binding.model.sha256!=config.model_sha256 || binding.tokenizer.sha256!=config.tokenizer_sha256 {
            return Err(EmbeddingError::AssetMismatch);
        }
        let deadline=tokio::time::Instant::now()+limits.timeout;
        let prepare=async {
            // Model/tokenizer use verified in-memory bytes, never adjacent external-data paths.
            let model=verified_bytes(&binding.model,2*1024*1024*1024).await?;
            let tokenizer=verified_bytes(&binding.tokenizer,64*1024*1024).await?;
            Ok::<_,EmbeddingError>((model,tokenizer))
        };
        let (model,tokenizer)=tokio::select! {biased;
            _=cancel.cancelled()=>return Err(EmbeddingError::Cancelled),
            result=tokio::time::timeout_at(deadline,prepare)=>result.map_err(|_|EmbeddingError::Timeout)??,
        };
        let dimensions=spec.dimensions;
        let (session,tokenizer)=native_setup(cancel,deadline,move|fenced| {
            let tokenizer=configure_tokenizer(&tokenizer,&config)?;
            // Copy and hash from one source handle into a fresh private read-only path.
            let runtime=ProtectedRuntime::prepare(binding.runtime)?;
            initialize_runtime(runtime,fenced,deadline)?;
            let session=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Session::builder().map_err(|_|EmbeddingError::RuntimeUnavailable)?
                    .with_execution_providers([ort::ep::CPU::default().build()]).map_err(|_|EmbeddingError::RuntimeUnavailable)?
                    .with_intra_threads(1).map_err(|_|EmbeddingError::RuntimeUnavailable)?
                    .with_inter_threads(1).map_err(|_|EmbeddingError::RuntimeUnavailable)?
                    .with_logger(Arc::new(|_,_,_,_,_|{})).map_err(|_|EmbeddingError::RuntimeUnavailable)?
                    .commit_from_memory(&model).map_err(|_|EmbeddingError::ModelContract)
            })).map_err(|_|EmbeddingError::RuntimeUnavailable)??;
            validate_session(&session,&config,dimensions)?;
            Ok((session,tokenizer))
        }).await?;
        Ok(Self{spec,tokenizer:Arc::new(tokenizer),session:Arc::new(tokio::sync::Mutex::new(session)),limits})
    }
}
fn configure_tokenizer(bytes:&[u8],config:&LocalModelConfig)->Result<Tokenizer,EmbeddingError> {
    let mut tokenizer=Tokenizer::from_bytes(bytes).map_err(|_|EmbeddingError::ModelContract)?;
    tokenizer.with_truncation(Some(TruncationParams{direction:TruncationDirection::Right,max_length:config.max_tokens,
        strategy:TruncationStrategy::LongestFirst,stride:0})).map_err(|_|EmbeddingError::ModelContract)?;
    tokenizer.with_padding(Some(PaddingParams{strategy:PaddingStrategy::BatchLongest,direction:PaddingDirection::Right,
        pad_to_multiple_of:None,pad_id:config.pad_id,pad_type_id:config.pad_type_id,pad_token:config.pad_token.clone()}));
    Ok(tokenizer)
}
struct SetupGuard(Arc<Mutex<bool>>);
impl Drop for SetupGuard {fn drop(&mut self){*self.0.lock()=true;}}
/// Native environment/session creation is not preemptible. Cancellation fences work
/// before it starts; once started, await quiescence before returning a cancelled status.
async fn native_setup<T:Send+'static>(cancel:CancellationToken,deadline:tokio::time::Instant,
    setup:impl FnOnce(&Mutex<bool>)->Result<T,EmbeddingError>+Send+'static)->Result<T,EmbeddingError> {
    if cancel.is_cancelled() {return Err(EmbeddingError::Cancelled);}
    if tokio::time::Instant::now()>=deadline {return Err(EmbeddingError::Timeout);}
    let fenced=Arc::new(Mutex::new(false));let guard=SetupGuard(fenced.clone());
    let mut worker=tokio::task::spawn_blocking(move|| {
        if *fenced.lock() {return Err(EmbeddingError::Cancelled);}
        setup(&fenced)
    });
    tokio::select! {biased;
        _=cancel.cancelled()=>{
            *guard.0.lock()=true;let _=worker.await;Err(EmbeddingError::Cancelled)
        },
        _=tokio::time::sleep_until(deadline)=>{
            *guard.0.lock()=true;let _=worker.await;Err(EmbeddingError::Timeout)
        },
        result=&mut worker=>result.map_err(|_|EmbeddingError::RuntimeUnavailable)?,
    }
}
fn initialize_runtime(runtime:ProtectedRuntime,fenced:&Mutex<bool>,deadline:tokio::time::Instant)->Result<(),EmbeddingError> {
    let mut current=RUNTIME.lock();
    if *fenced.lock() {return Err(EmbeddingError::Cancelled);}
    if tokio::time::Instant::now()>=deadline {return Err(EmbeddingError::Timeout);}
    if let Some(existing)=current.as_ref() {
        if existing.approved!=runtime.original || !existing.committed {return Err(EmbeddingError::RuntimeConflict);}
        return Ok(());
    }
    let builder=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||ort::init_from(&runtime.path)))
        .map_err(|_|EmbeddingError::RuntimeUnavailable)?.map_err(|_|EmbeddingError::RuntimeUnavailable)?;
    // dlopen is already process-global. Retain its exact protected bytes even if
    // environment commit fails; never pretend another library can replace it.
    *current=Some(RuntimeState{approved:runtime.original.clone(),_protected:runtime,committed:false});
    let committed=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||builder
        .with_telemetry(false).with_logger(Arc::new(|_,_,_,_,_|{}))
        .with_execution_providers([ort::ep::CPU::default().build()]).commit()))
        .map_err(|_|EmbeddingError::RuntimeUnavailable)?;
    if !committed {return Err(EmbeddingError::RuntimeConflict);}
    current.as_mut().expect("loaded runtime retained").committed=true;Ok(())
}
fn validate_session(session:&Session,config:&LocalModelConfig,dimensions:usize)->Result<(),EmbeddingError> {
    let mut expected=BTreeSet::from([config.tensors.input_ids.as_str(),config.tensors.attention_mask.as_str()]);
    if let Some(name)=&config.tensors.token_type_ids {expected.insert(name.as_str());}
    if session.inputs().len()!=expected.len() {return Err(EmbeddingError::ModelContract);}
    for input in session.inputs() {
        if !expected.contains(input.name()) {return Err(EmbeddingError::ModelContract);}
        match input.dtype() {
            ValueType::Tensor{ty:TensorElementType::Int64,shape,..} if shape.len()==2
                && shape.iter().all(|&s|s == -1 || s>0)=>{},
            _=>return Err(EmbeddingError::ModelContract),
        }
    }
    let output=session.outputs().iter().find(|o|o.name()==config.tensors.output).ok_or(EmbeddingError::ModelContract)?;
    let rank=if config.pooling==Pooling::Pooled {2}else {3};
    match output.dtype() {
        ValueType::Tensor{ty:TensorElementType::Float32,shape,..} if shape.len()==rank
            && shape.last().is_some_and(|&d|d == -1 || d==dimensions as i64)=>Ok(()),
        _=>Err(EmbeddingError::ModelContract),
    }
}
struct TerminateOnDrop(Arc<RunOptions>);
impl Drop for TerminateOnDrop {fn drop(&mut self){let _=self.0.terminate();}}
#[async_trait::async_trait]
impl Embedding for LocalEmbedding {
    fn fingerprint(&self)->&EmbeddingFingerprint {self.spec.fingerprint()}
    async fn embed(&self,purpose:EmbeddingPurpose,texts:&[String],cancel:CancellationToken)->Result<EmbeddingBatch,EmbeddingError> {
        let prepared=self.spec.prepare(purpose,texts,&self.limits)?;
        let spec=self.spec.clone();let config=spec.local.clone().ok_or(EmbeddingError::InvalidProfile)?;
        let tokenizer=self.tokenizer.clone();let session=self.session.clone();
        let work=async {
            // Own lock remains held until the blocking native call actually finishes,
            // even when the async caller is dropped/cancelled or times out.
            let mut session=session.lock_owned().await;
            let options=Arc::new(RunOptions::new().map_err(|_|EmbeddingError::RuntimeUnavailable)?);
            let _terminate=TerminateOnDrop(options.clone());
            tokio::task::spawn_blocking(move|| {
                let count=prepared.len();
                let encodings=tokenizer.encode_batch(prepared,config.add_special_tokens).map_err(|_|EmbeddingError::ModelContract)?;
                let tokens=encodings.first().ok_or(EmbeddingError::ModelContract)?.len();
                if tokens==0 || tokens>config.max_tokens || encodings.len()!=count || encodings.iter().any(|e|e.len()!=tokens) {
                    return Err(EmbeddingError::ModelContract);
                }
                let ids=encodings.iter().flat_map(|e|e.get_ids().iter().copied().map(i64::from)).collect::<Vec<_>>();
                let mask=encodings.iter().flat_map(|e|e.get_attention_mask().iter().copied().map(i64::from)).collect::<Vec<_>>();
                let shape=[count,tokens];
                let mut inputs=ort::inputs![
                    config.tensors.input_ids.as_str()=>Tensor::from_array((shape,ids)).map_err(|_|EmbeddingError::ModelContract)?,
                    config.tensors.attention_mask.as_str()=>TensorRef::from_array_view((shape,mask.as_slice())).map_err(|_|EmbeddingError::ModelContract)?
                ];
                if let Some(name)=&config.tensors.token_type_ids {
                    let types=encodings.iter().flat_map(|e|e.get_type_ids().iter().copied().map(i64::from)).collect::<Vec<_>>();
                    inputs.push((name.as_str().into(),Tensor::from_array((shape,types)).map_err(|_|EmbeddingError::ModelContract)?.into()));
                }
                let outputs=session.run_with_options(inputs,&options).map_err(|_|EmbeddingError::ModelContract)?;
                let output=outputs.get(&config.tensors.output).ok_or(EmbeddingError::ModelContract)?;
                let (shape,values)=output.try_extract_tensor::<f32>().map_err(|_|EmbeddingError::ModelContract)?;
                let vectors=pool_output(values,shape,&mask,count,tokens,spec.dimensions,config.pooling)?;
                spec.validate_vectors(vectors,count)
            }).await.map_err(|_|EmbeddingError::RuntimeUnavailable)?
        };
        tokio::select! {biased;
            _=cancel.cancelled()=>Err(EmbeddingError::Cancelled),
            result=tokio::time::timeout(self.limits.timeout,work)=>result.map_err(|_|EmbeddingError::Timeout)?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn missing_approved_files_report_setup_without_implicit_runtime_search() {
        let config=LocalModelConfig{model_sha256:"a".repeat(64),tokenizer_sha256:"b".repeat(64),max_tokens:32,pooling:Pooling::Mean,
            tensors:TensorNames{input_ids:"input_ids".into(),attention_mask:"attention_mask".into(),token_type_ids:None,output:"last_hidden_state".into()},
            add_special_tokens:true,pad_id:0,pad_type_id:0,pad_token:"[PAD]".into()};
        let profile:EmbeddingProfile=serde_json::from_value(serde_json::json!({"mode":"local_cpu","local":config,"model":"fixture","revision":"1",
            "dimensions":2,"normalization":"l2","distance":"cosine","document_prefix":"","query_prefix":""})).unwrap();
        let asset=|hash:&str|BoundAsset{path:std::env::temp_dir().join(format!("not-supplied-{}",uuid::Uuid::new_v4())),bytes:1,sha256:hash.repeat(64)};
        let binding=LocalBinding{runtime:asset("c"),model:asset("a"),tokenizer:asset("b")};
        match LocalEmbedding::load(&profile,binding,EmbeddingLimits::default(),CancellationToken::new()).await {
            Err(e)=>assert_eq!(e,EmbeddingError::SetupRequired),Ok(_)=>panic!("missing assets accepted"),
        }
    }
    #[test]
    fn real_tokenizer_truncates_and_pads_with_exact_mask() {
        let config=LocalModelConfig{model_sha256:"a".repeat(64),tokenizer_sha256:"b".repeat(64),max_tokens:3,pooling:Pooling::Mean,
            tensors:TensorNames{input_ids:"input_ids".into(),attention_mask:"attention_mask".into(),token_type_ids:None,output:"hidden".into()},
            add_special_tokens:false,pad_id:1,pad_type_id:0,pad_token:"[PAD]".into()};
        let tokenizer=configure_tokenizer(include_bytes!("fixtures/tiny-tokenizer.json"),&config).unwrap();
        let encoded=tokenizer.encode_batch(vec!["a b c d","b"],false).unwrap();
        assert_eq!(encoded[0].get_ids(),&[2,3,4]);
        assert_eq!(encoded[1].get_ids(),&[3,1,1]);
        assert_eq!(encoded[1].get_attention_mask(),&[1,0,0]);
        assert!(configure_tokenizer(b"invalid tokenizer",&config).is_err());
    }
    #[tokio::test]
    async fn cancelled_setup_never_begins_commit_and_started_commit_is_awaited() {
        use std::sync::atomic::{AtomicBool,Ordering};
        let called=Arc::new(AtomicBool::new(false));let call=called.clone();
        let cancel=CancellationToken::new();cancel.cancel();
        assert_eq!(native_setup(cancel,tokio::time::Instant::now()+std::time::Duration::from_secs(1),
            move|_|{call.store(true,Ordering::SeqCst);Ok(())}).await.unwrap_err(),EmbeddingError::Cancelled);
        assert!(!called.load(Ordering::SeqCst));
        let (started_tx,started_rx)=tokio::sync::oneshot::channel();
        let (release_tx,release_rx)=std::sync::mpsc::channel();
        let completed=Arc::new(AtomicBool::new(false));let complete=completed.clone();
        let cancel=CancellationToken::new();let worker_cancel=cancel.clone();
        let mut result=tokio::spawn(native_setup(worker_cancel,tokio::time::Instant::now()+std::time::Duration::from_secs(2),
            move|_|{started_tx.send(()).unwrap();release_rx.recv().unwrap();complete.store(true,Ordering::SeqCst);Ok(())}));
        started_rx.await.unwrap();cancel.cancel();
        // A reported cancellation must not race a still-running process-global commit.
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20),&mut result).await.is_err());
        assert!(!completed.load(Ordering::SeqCst));release_tx.send(()).unwrap();
        assert_eq!(result.await.unwrap().unwrap_err(),EmbeddingError::Cancelled);
        assert!(completed.load(Ordering::SeqCst));
    }
}
