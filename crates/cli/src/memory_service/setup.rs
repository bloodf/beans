//! Typed user-only setup and one-use approvals. Never reachable from model tools.
use std::{collections::{BTreeSet, HashMap}, path::PathBuf, sync::Arc, time::{Duration, Instant}};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use crate::{app::App, embeddings::{self, assets::{AssetInstaller, AssetPlan, LocalBinding}, Embedding, EmbeddingMode}};
use super::{dispatch::{admit, recheck, TurnAccess}, types::*};

#[derive(Default)]
pub struct SetupRuntime {
    installers: Mutex<HashMap<String, Arc<AssetInstaller>>>,
    approvals: Mutex<HashMap<String, Approval>>,
    embeddings: Mutex<HashMap<String, (Revision, Arc<dyn Embedding>)>>,
    calls: Mutex<HashMap<String,CancellationToken>>,
    #[cfg(feature = "memory-pgvector")]
    pgvector: Mutex<HashMap<String, Arc<super::backends::pgvector::PgvectorBackend>>>,
    #[cfg(all(feature = "memory-pgvector", feature = "memory-lance"))]
    lance: Mutex<HashMap<String, Arc<super::backends::lance::LanceBackend>>>,
}
impl SetupRuntime {
    pub fn clear(&self) {
        for token in self.calls.lock().values() { token.cancel(); }
        self.installers.lock().clear(); self.approvals.lock().clear(); self.embeddings.lock().clear();
        #[cfg(feature = "memory-pgvector")] self.pgvector.lock().clear();
        #[cfg(all(feature = "memory-pgvector", feature = "memory-lance"))] self.lance.lock().clear();
    }
}
struct Approval {
    account: String, actor: String, expires: Instant, bot_id: Option<String>, scope: Option<MemoryScope>,
    profile_id: Option<String>, profile_revision: Option<Revision>, action: Action,
}
enum Action {
    Assets { installer: Arc<AssetInstaller>, token: String, digest: String },
    #[cfg(feature = "memory-pgvector")]
    Pgvector { backend: Arc<super::backends::pgvector::PgvectorBackend>, token: String },
    #[cfg(all(feature = "memory-pgvector", feature = "memory-lance"))]
    LanceBind { directory: PathBuf, create: bool },
    #[cfg(all(feature = "memory-pgvector", feature = "memory-lance"))]
    LanceExport { transfer: super::backends::lance::LanceTransfer, destination: PathBuf },
    #[cfg(all(feature = "memory-pgvector", feature = "memory-lance"))]
    LanceImport { backend: Arc<super::backends::lance::LanceBackend>, transfer: super::backends::lance::LanceTransfer },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedLocal { profile_revision: Revision, binding: LocalBinding }
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedLance { connection_revision: Revision, directory: PathBuf, runner_id: String }
fn decode<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, MemoryError> {
    serde_json::from_value(value).map_err(|_| MemoryError::new("invalid_setup_request"))
}
fn account(app: &App) -> Result<String, MemoryError> {
    app.machine_file().map(|m|m.identity_pubkey).ok_or_else(||MemoryError::new("identity_required"))
}
fn profile(app: &App, id: &str) -> Result<(Revision, EmbeddingProfile), MemoryError> {
    let config = app.memory_config.lock();
    let record = config.embeddings.get(id).ok_or_else(||MemoryError::new("profile_not_found"))?;
    Ok((record.revision.clone(),record.value.clone().ok_or_else(||MemoryError::new("profile_not_found"))?))
}
fn key(app: &App, kind: &str, id: &str) -> Result<String, MemoryError> {
    Ok(format!("{kind}:{}",namespace(&account(app)?,id)?))
}
fn save_binding(app: &App, kind: &str, id: &str, value: &impl Serialize) -> Result<(), MemoryError> {
    app.store.save_memory_binding(&key(app,kind,id)?, &serde_json::to_string(value).map_err(|_|MemoryError::new("invalid_binding"))?)
        .map_err(|_|MemoryError::new("memory_storage_failed"))
}
fn load_binding<T: for<'de> Deserialize<'de>>(app: &App, kind: &str, id: &str) -> Result<T, MemoryError> {
    let value = app.store.memory_binding(&key(app,kind,id)?).map_err(|_|MemoryError::new("memory_storage_failed"))?
        .ok_or_else(||MemoryError::new("runner_binding_required"))?;
    serde_json::from_str(&value).map_err(|_|MemoryError::new("invalid_binding"))
}
fn preview(app: &App, actor: &str, bot: Option<&TurnAccess>, profile: Option<(&str,&Revision)>, action: Action,
    name: &str, details: Value, ttl: u64) -> Result<Value, MemoryError> {
    let runtime = &app.memory_runtime.setup;
    let token = uuid::Uuid::new_v4().to_string();
    let mut approvals=runtime.approvals.lock();
    approvals.retain(|_,a|a.expires>Instant::now());
    if approvals.len()>=32 {return Err(MemoryError::new("too_many_approvals"));}
    approvals.insert(token.clone(),Approval { account:account(app)?,actor:actor.into(),expires:Instant::now()+Duration::from_secs(ttl),
        bot_id:bot.map(|b|b.bot_id.clone()),scope:bot.map(|b|b.scope.clone()),profile_id:profile.map(|(id,_)|id.into()),
        profile_revision:profile.map(|(_,r)|r.clone()),action });
    Ok(json!({"token":token,"expires_at":crate::config::now_secs()+ttl as f64,"action":name,
        "runner_id":app.this_device_id(),"bot_id":bot.map(|b|&b.bot_id),"profile_id":profile.map(|(id,_)|id),
        "connection_revision":bot.map(|b|&b.scope.connection_revision),"profile_revision":profile.map(|(_,r)|r),"details":details}))
}

/// Explicit constructor wiring: no factory, fake adapter or automatic schema creation.
pub async fn initialize(app: &Arc<App>, access: &TurnAccess, cancel: CancellationToken) -> Result<(), MemoryError> {
    recheck(app,access,false,&cancel)?;
    let connection=app.memory_config.lock().connections.get(&access.scope.connection_id).and_then(|r|r.value.clone())
        .ok_or_else(||MemoryError::new("connection_disconnected"))?;
    if connection.backend==BackendKind::LanceDb && connection.endpoint.is_some() {
        return Err(MemoryError::new("lancedb_cloud_transport_unavailable"));
    }
    match connection.backend {
        BackendKind::Hindsight => {
            let backend=super::backends::hindsight::Hindsight::connect(&connection,&access.scope,cancel.clone()).await?;
            recheck(app,access,false,&cancel)?;app.memory_runtime.register(access.scope.clone(),Arc::new(backend));
        },
        BackendKind::OpenViking => {
            let backend=super::backends::openviking::OpenViking::connect(&connection,&access.scope,cancel.clone()).await?;
            recheck(app,access,false,&cancel)?;app.memory_runtime.register(access.scope.clone(),Arc::new(backend));
        },
        #[cfg(feature="memory-pgvector")]
        BackendKind::Pgvector => {
            use super::backends::pgvector::{PgvectorBackend,PgvectorConfig};
            let (schema,role)=match &connection.options {Some(BackendOptions::Pgvector{schema,role})=>(schema,role),_=>return Err(MemoryError::new("postgres_options_required"))};
            let (space,embedding)=vector_space(app,&connection,cancel.clone()).await?;
            let mut config=PgvectorConfig::new(connection.endpoint.as_deref().ok_or_else(||MemoryError::new("endpoint_required"))?,schema,space)?
                .with_password(connection.secret.as_deref())?;
            if let Some(role)=role { config=config.with_user(role)?; }
            let backend=Arc::new(PgvectorBackend::connect(config,access.scope.clone(),Some(embedding),cancel.clone()).await?);
            recheck(app,access,false,&cancel)?;
            app.memory_runtime.setup.pgvector.lock().insert(access.scope.namespace.clone(),backend.clone());
            app.memory_runtime.register(access.scope.clone(),backend);
        },
        #[cfg(all(feature="memory-pgvector",feature="memory-lance"))]
        BackendKind::LanceDb => {
            use super::backends::lance::{LanceBackend,LanceBinding};
            let (space,embedding)=vector_space(app,&connection,cancel.clone()).await?;
            let binding=if let Some(uri)=&connection.endpoint {
                let region=match &connection.options {Some(BackendOptions::LanceDb{region:Some(region)})=>region.clone(),_=>return Err(MemoryError::new("lance_region_required"))};
                LanceBinding::Cloud{uri:uri.clone(),region,api_key:connection.secret.clone().ok_or_else(||MemoryError::new("secret_required"))?}
            } else {
                let saved:SavedLance=load_binding(app,"lance",&access.bot_id)?;
                if saved.connection_revision!=access.scope.connection_revision||Some(&saved.runner_id)!=app.this_device_id().as_ref(){return Err(MemoryError::new("runner_binding_stale"));}
                LanceBinding::Local{directory:saved.directory,runner_id:saved.runner_id}
            };
            let backend=Arc::new(LanceBackend::open(binding,access.scope.clone(),space,Some(embedding),false,cancel.clone()).await?);
            recheck(app,access,false,&cancel)?;
            app.memory_runtime.setup.lance.lock().insert(access.scope.namespace.clone(),backend.clone());
            app.memory_runtime.register(access.scope.clone(),backend);
        },
        #[allow(unreachable_patterns)]
        _=>return Err(MemoryError::new("adapter_not_in_build")),
    }
    Ok(())
}
#[cfg(feature="memory-pgvector")]
async fn vector_space(app:&Arc<App>,connection:&Connection,cancel:CancellationToken)->Result<(super::backends::pgvector::VectorSpace,Arc<dyn Embedding>),MemoryError>{
    use super::backends::pgvector::{VectorSpace,VectorDistance};
    let id=connection.embedding_profile.as_deref().ok_or_else(||MemoryError::new("embedding_profile_required"))?;
    let (revision,p)=profile(app,id)?;
    let spec=embeddings::EmbeddingSpec::from_profile(&p).map_err(|_|MemoryError::new("invalid_embedding_profile"))?;
    let embedding:Arc<dyn Embedding>=match p.mode {
        EmbeddingMode::Api=>Arc::new(embeddings::api::ApiEmbedding::new(&p,connection.allow_insecure_http,embeddings::EmbeddingLimits::default()).map_err(|_|MemoryError::new("invalid_embedding_profile"))?),
        EmbeddingMode::LocalCpu=>{
            let registered=app.memory_runtime.setup.embeddings.lock();
            registered.get(id).filter(|(r,_)|r==&revision).map(|(_,e)|e.clone()).ok_or_else(||MemoryError::new("local_embedding_setup_required"))?
        },
    };
    if cancel.is_cancelled(){return Err(MemoryError::new("cancelled"));}
    let distance=match p.distance.as_str(){"cosine"=>VectorDistance::Cosine,"dot"=>VectorDistance::Dot,"euclidean"=>VectorDistance::Euclidean,_=>return Err(MemoryError::new("invalid_embedding_profile"))};
    Ok((VectorSpace::new(spec.fingerprint().as_str().into(),p.dimensions as usize,distance)?,embedding))
}


pub async fn serve(app:&Arc<App>,actor:&str,method:&str,params:Value)->Result<Value,MemoryError>{
    let expected_account=account(app)?;
    let expected_clock=app.memory_config.lock().clock;
    let cancel=CancellationToken::new();
    let id=uuid::Uuid::new_v4().to_string();
    app.memory_runtime.setup.calls.lock().insert(id.clone(),cancel.clone());
    struct Call<'a>{runtime:&'a SetupRuntime,id:String}
    impl Drop for Call<'_>{fn drop(&mut self){self.runtime.calls.lock().remove(&self.id);}}
    let _call=Call{runtime:&app.memory_runtime.setup,id};
    let future=serve_inner(app,actor,method,params,cancel.clone());tokio::pin!(future);
    let mut tick=tokio::time::interval(Duration::from_millis(25));
    loop {tokio::select!{
        _=tick.tick()=>if app.is_paused()||account(app).ok().as_ref()!=Some(&expected_account)||app.memory_config.lock().clock!=expected_clock {cancel.cancel();},
        result=&mut future=>return result,
    }}
}
async fn serve_inner(app:&Arc<App>,actor:&str,method:&str,params:Value,cancel:CancellationToken)->Result<Value,MemoryError>{
    let _admission=app.update.try_admit().ok_or_else(||MemoryError::new("runner_draining"))?;
    if app.is_paused(){return Err(MemoryError::new("account_paused"));}
    if method.ends_with(".apply") {
        #[derive(Deserialize)] #[serde(deny_unknown_fields)]
        struct Apply { token: String, confirm: bool, bot_id: String }
        #[derive(Deserialize)] #[serde(deny_unknown_fields)]
        struct AssetsApply { preview_token: String, preview_digest: String, confirm: bool }
        let (token, confirmed, bot_id, supplied_digest) = if method == "memory.embeddings.local.apply" {
            let p: AssetsApply = decode(params)?;
            (p.preview_token,p.confirm,None,Some(p.preview_digest))
        } else {
            let p: Apply = decode(params)?;
            (p.token,p.confirm,Some(p.bot_id),None)
        };
        if !confirmed { return Err(MemoryError::new("confirmation_required")); }
        let approval=app.memory_runtime.setup.approvals.lock().remove(&token).ok_or_else(||MemoryError::new("approval_not_found"))?;
        if approval.expires<=Instant::now(){return Err(MemoryError::new("approval_expired"));}
        if approval.account!=account(app)?||approval.actor!=actor||approval.bot_id!=bot_id{return Err(MemoryError::new("approval_stale"));}
        if let (Some(id),Some(revision))=(&approval.profile_id,&approval.profile_revision){if &profile(app,id)?.0!=revision{return Err(MemoryError::new("approval_stale"));}}
        let access=approval.bot_id.as_ref().map(|id|admit(app,id,false,&cancel)).transpose()?;
        if access.as_ref().map(|a|&a.scope)!=approval.scope.as_ref(){return Err(MemoryError::new("approval_stale"));}
        match approval.action {
            Action::Assets{installer,token,digest} if method=="memory.embeddings.local.apply"=>{
                if supplied_digest.as_deref()!=Some(&digest){return Err(MemoryError::new("approval_stale"));}
                let id=approval.profile_id.ok_or_else(||MemoryError::new("approval_stale"))?;
                let (revision,p)=profile(app,&id)?;
                let binding=installer.apply(&token,&digest,true,cancel.clone()).await.map_err(|_|MemoryError::new("asset_install_failed"))?;
                validate_binding(&p,&binding)?;
                if profile(app,&id)?.0!=revision||account(app)?!=approval.account{return Err(MemoryError::new("approval_stale"));}
                save_binding(app,"embedding",&id,&SavedLocal{profile_revision:revision.clone(),binding:binding.clone()})?;
                #[cfg(feature="embedding-local")]
                {
                    let embedding=embeddings::local::LocalEmbedding::load(&p,binding,embeddings::EmbeddingLimits::default(),cancel).await.map_err(|_|MemoryError::new("runtime_unavailable"))?;
                    if profile(app,&id)?.0!=revision{return Err(MemoryError::new("approval_stale"));}
                    app.memory_runtime.setup.embeddings.lock().insert(id,(revision,Arc::new(embedding)));
                    return Ok(json!({"installed":true,"status":"ready"}));
                }
                #[cfg(not(feature="embedding-local"))]
                return Ok(json!({"installed":true,"status":"runtime_unavailable"}));
            },
            #[cfg(feature="memory-pgvector")]
            Action::Pgvector{backend,token} if method=="memory.pgvector.initialize.apply"=>{
                backend.initialize_apply(&token,cancel).await?;return Ok(json!({"initialized":true}));
            },
            #[cfg(all(feature="memory-pgvector",feature="memory-lance"))]
            Action::LanceBind{directory,create} if method=="memory.lance.binding.apply"=>{
                let a=access.ok_or_else(||MemoryError::new("approval_stale"))?;
                let connection=app.memory_config.lock().connections[&a.scope.connection_id].value.clone().ok_or_else(||MemoryError::new("connection_disconnected"))?;
                let (space,embedding)=vector_space(app,&connection,cancel.clone()).await?;
                let runner_id=app.this_device_id().ok_or_else(||MemoryError::new("identity_required"))?;
                let backend=Arc::new(super::backends::lance::LanceBackend::open(super::backends::lance::LanceBinding::Local{directory:directory.clone(),runner_id:runner_id.clone()},a.scope.clone(),space,Some(embedding),create,cancel.clone()).await?);
                recheck(app,&a,false,&cancel)?;
                save_binding(app,"lance",&a.bot_id,&SavedLance{connection_revision:a.scope.connection_revision.clone(),directory,runner_id})?;
                app.memory_runtime.setup.lance.lock().insert(a.scope.namespace.clone(),backend.clone());app.memory_runtime.register(a.scope,backend);
                return Ok(json!({"status":"ready"}));
            },
            #[cfg(all(feature="memory-pgvector",feature="memory-lance"))]
            Action::LanceExport{transfer,destination} if method=="memory.lance.export.apply"=>{
                let bytes=serde_json::to_vec(&transfer).map_err(|_|MemoryError::new("invalid_transfer"))?;
                let mut options=tokio::fs::OpenOptions::new();options.write(true).create_new(true);
                #[cfg(unix)] options.mode(0o600);
                let mut file=options.open(destination).await.map_err(|_|MemoryError::new("export_target_unavailable"))?;
                use tokio::io::AsyncWriteExt;
                file.write_all(&bytes).await.map_err(|_|MemoryError::new("export_failed"))?;file.sync_all().await.map_err(|_|MemoryError::new("export_failed"))?;
                return Ok(json!({"exported":true,"bytes":bytes.len()}));
            },
            #[cfg(all(feature="memory-pgvector",feature="memory-lance"))]
            Action::LanceImport{backend,transfer} if method=="memory.lance.import.apply"=>{
                let a=access.ok_or_else(||MemoryError::new("approval_stale"))?;
                if app.store.memory_fence(&a.bot_id)?.is_some_and(|f|f.pending){return Err(MemoryError::new("deletion_pending"));}
                backend.import(&a.scope,transfer,true,cancel).await?;return Ok(json!({"imported":true}));
            },
            _=>return Err(MemoryError::new("approval_action_mismatch")),
        }
    }
    if method.starts_with("memory.embeddings.local.") {
        #[derive(Deserialize)] #[serde(deny_unknown_fields)]
        struct Preview{profile_id:String,profile_revision:Revision,plan:AssetPlan}
        #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Status{profile_id:String}
        if method.ends_with("status") {
            let p:Status=decode(params)?;let (revision,embedding_profile)=profile(app,&p.profile_id)?;
            let ready=app.memory_runtime.setup.embeddings.lock().get(&p.profile_id).is_some_and(|(r,_)|r==&revision);
            if ready { return Ok(json!({"status":"ready"})); }
            let saved: SavedLocal = match load_binding(app,"embedding",&p.profile_id) {
                Ok(saved) => saved, Err(_) => return Ok(json!({"status":"setup_required"})),
            };
            if saved.profile_revision!=revision { return Ok(json!({"status":"setup_required"})); }
            validate_binding(&embedding_profile,&saved.binding)?;
            #[cfg(feature="embedding-local")]
            {
                let embedding=embeddings::local::LocalEmbedding::load(&embedding_profile,saved.binding,embeddings::EmbeddingLimits::default(),cancel).await.map_err(|_|MemoryError::new("runtime_unavailable"))?;
                if profile(app,&p.profile_id)?.0!=revision { return Err(MemoryError::new("approval_stale")); }
                app.memory_runtime.setup.embeddings.lock().insert(p.profile_id,(revision,Arc::new(embedding)));
                return Ok(json!({"status":"ready"}));
            }
            #[cfg(not(feature="embedding-local"))]
            return Ok(json!({"status":"runtime_unavailable"}));
        }
        let p:Preview=decode(params)?;let (revision,profile)=profile(app,&p.profile_id)?;
        if revision!=p.profile_revision||profile.mode!=EmbeddingMode::LocalCpu{return Err(MemoryError::new("approval_stale"));}
        let local=profile.local.as_ref().ok_or_else(||MemoryError::new("invalid_embedding_profile"))?;
        for asset in &p.plan.assets {match asset.kind {
            embeddings::assets::AssetKind::Model if asset.sha256!=local.model_sha256=>return Err(MemoryError::new("asset_mismatch")),
            embeddings::assets::AssetKind::Tokenizer if asset.sha256!=local.tokenizer_sha256=>return Err(MemoryError::new("asset_mismatch")),_=>{}}}
        let account=account(app)?;
        let installer={let mut installers=app.memory_runtime.setup.installers.lock();
            if let Some(i)=installers.get(&account){i.clone()}else{
                // Administrator-controlled exact origins, never copied from RPC/model data.
                let origins=std::env::var("BEANS_MEMORY_ASSET_ORIGINS").unwrap_or_default().split(',').filter(|s|!s.is_empty()).map(str::to_owned).collect::<BTreeSet<_>>();
                let root=app.config.home.join("memory-assets").join(namespace(&account,"assets")?);
                let i=Arc::new(AssetInstaller::new(root,origins,false).map_err(|_|MemoryError::new("invalid_asset_policy"))?);installers.insert(account,i.clone());i}};
        let result=installer.preview(p.plan).map_err(|_|MemoryError::new("invalid_asset_plan"))?;
        let response=preview(app,actor,None,Some((&p.profile_id,&revision)),Action::Assets{installer,token:result.token,digest:result.digest.clone()},method,Value::Null,600)?;
        return Ok(json!({"preview_token":response["token"],"preview_digest":result.digest,
            "expires_in_seconds":600,"runner_id":app.this_device_id(),"profile_id":p.profile_id,"profile_revision":revision,
            "total_bytes":result.total_bytes,"assets":result.assets}));
    }
    let bot_id=params["bot_id"].as_str().ok_or_else(||MemoryError::new("missing_bot_id"))?.to_owned();
    let access=admit(app,&bot_id,false,&cancel)?;
    if method=="memory.lance.binding.preview" {
        #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Bind{bot_id:String,directory:PathBuf,create:bool}
        let p:Bind=decode(params)?;
        if p.bot_id!=bot_id||!p.directory.is_absolute(){return Err(MemoryError::new("invalid_binding"));}
        #[cfg(all(feature="memory-pgvector",feature="memory-lance"))]
        {return preview(app,actor,Some(&access),None,Action::LanceBind{directory:p.directory.clone(),create:p.create},method,
            json!({"directory":p.directory,"create":p.create,"table":"beans_memory_v1","namespace":access.scope.namespace}),300);}
    }
    if app.memory_runtime.backend(&access.scope).is_err(){initialize(app,&access,cancel.clone()).await?;}
    #[cfg(feature="memory-pgvector")]
    if method=="memory.pgvector.initialize.preview"{
        #[derive(Deserialize)] #[serde(deny_unknown_fields)]struct Bot{bot_id:String}
        let p:Bot=decode(params.clone())?;if p.bot_id!=bot_id{return Err(MemoryError::new("invalid_setup_request"));}
        let backend=app.memory_runtime.setup.pgvector.lock().get(&access.scope.namespace).cloned().ok_or_else(||MemoryError::new("backend_mismatch"))?;
        let result=backend.initialize_preview(cancel.clone()).await?;
        return preview(app,actor,Some(&access),None,Action::Pgvector{backend,token:result.approval_token},method,
            json!({"target":result.target,"schema":result.schema,"sql":result.sql,"readiness":result.readiness}),300);
    }
    #[cfg(all(feature="memory-pgvector",feature="memory-lance"))]
    if matches!(method,"memory.lance.export.preview"|"memory.lance.import.preview"){
        #[derive(Deserialize)]#[serde(deny_unknown_fields)]struct Transfer{bot_id:String,path:PathBuf}
        let p:Transfer=decode(params)?;if p.bot_id!=bot_id||!p.path.is_absolute(){return Err(MemoryError::new("invalid_transfer_path"));}
        let backend=app.memory_runtime.setup.lance.lock().get(&access.scope.namespace).cloned().ok_or_else(||MemoryError::new("backend_mismatch"))?;
        let export=method.contains("export");
        let transfer=if export{backend.export(&access.scope,cancel.clone()).await?}else{
            let size=tokio::fs::metadata(&p.path).await.map_err(|_|MemoryError::new("transfer_unavailable"))?.len();
            if size>8*1024*1024{return Err(MemoryError::new("transfer_too_large"));}
            let bytes=tokio::fs::read(&p.path).await.map_err(|_|MemoryError::new("transfer_unavailable"))?;
            if bytes.len()>8*1024*1024{return Err(MemoryError::new("transfer_too_large"));}
            serde_json::from_slice(&bytes).map_err(|_|MemoryError::new("invalid_transfer"))?};
        if transfer.namespace!=access.scope.namespace{return Err(MemoryError::new("transfer_scope_mismatch"));}
        let details=json!({"path":p.path,"namespace":transfer.namespace,"space":transfer.space,"documents":transfer.rows.len(),
            "sha256":digest(&serde_json::to_vec(&transfer).map_err(|_|MemoryError::new("invalid_transfer"))?)});
        let action=if export{Action::LanceExport{transfer,destination:p.path}}else{Action::LanceImport{backend,transfer}};
        return preview(app,actor,Some(&access),None,action,method,details,300);
    }
    Err(MemoryError::new("setup_not_in_build"))
}
fn validate_binding(profile:&EmbeddingProfile,binding:&LocalBinding)->Result<(),MemoryError>{
    let local=profile.local.as_ref().ok_or_else(||MemoryError::new("invalid_embedding_profile"))?;
    if profile.mode!=EmbeddingMode::LocalCpu||local.model_sha256!=binding.model.sha256||local.tokenizer_sha256!=binding.tokenizer.sha256{return Err(MemoryError::new("asset_mismatch"));}Ok(())
}
