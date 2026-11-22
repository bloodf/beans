//! Runner-only, explicit acquisition. Preview never performs network I/O or loads code.
use super::*;
use serde::{Deserialize,Serialize};
use sha2::{Digest,Sha256};
use std::{collections::{BTreeSet,HashMap},path::{Path,PathBuf},time::{Duration,Instant}};
use parking_lot::Mutex;
use tokio::{io::{AsyncReadExt,AsyncWriteExt},fs};
use tokio_util::sync::CancellationToken;

#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum AssetKind { Runtime,Model,Tokenizer,
    /// A static embedding table (safetensors), read without a native runtime.
    Weights,
    /// An image model's vision tower (ONNX), beside its text tower in `Model`.
    ImageModel }
impl AssetKind {
    pub fn filename(self)->&'static str {
        match self {Self::Model=>"model.onnx",Self::ImageModel=>"image_model.onnx",Self::Tokenizer=>"tokenizer.json",Self::Weights=>"model.safetensors",Self::Runtime=>{
            #[cfg(target_os="windows")] {"onnxruntime.dll"}
            #[cfg(target_os="macos")] {"libonnxruntime.dylib"}
            #[cfg(not(any(target_os="macos",target_os="windows")))] {"libonnxruntime.so"}
        }}
    }
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum AssetSource { Supplied {path:PathBuf},Download {url:String},
    /// One file (`member`) of an archive downloaded from `url` and pinned by its own `bytes` and
    /// `sha256`: the ONNX Runtime library in its official wheel. The asset's size and hash are
    /// the member's.
    Archive {url:String,bytes:u64,sha256:String,member:String} }
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRequest {pub kind:AssetKind,pub source:AssetSource,pub license:String,pub bytes:u64,pub sha256:String}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetPlan {pub assets:Vec<AssetRequest>}
#[derive(Clone,Debug,Serialize)]
pub struct AssetPreview {pub token:String,pub digest:String,pub expires_in_seconds:u64,pub total_bytes:u64,pub assets:Vec<AssetRequest>}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundAsset {pub path:PathBuf,pub bytes:u64,pub sha256:String}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalBinding {pub runtime:BoundAsset,pub model:BoundAsset,pub tokenizer:BoundAsset}
/// What an approved plan installed, one asset per kind, in the plan's order.
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledAssets {pub assets:Vec<(AssetKind,BoundAsset)>}
impl InstalledAssets {
    pub fn get(&self,kind:AssetKind)->Option<&BoundAsset> {self.assets.iter().find(|(k,_)|*k==kind).map(|(_,a)|a)}
}
/// Where an approved download may be redirected, besides nowhere. Hugging Face answers a file
/// URL with a redirect to its CDN; the pinned size and SHA-256 still decide what is installed.
#[derive(Clone,Debug,Default,PartialEq,Eq)]
pub struct RedirectHosts {
    /// Hosts allowed as they are, like `huggingface.co`.
    pub exact:BTreeSet<String>,
    /// Allowed parent domains, like `hf.co`: `cdn-lfs.hf.co` matches, `evilhf.co` does not.
    pub parents:BTreeSet<String>,
}
impl RedirectHosts {
    pub(crate) fn allows(&self,url:&reqwest::Url,allow_insecure_http:bool)->bool {
        if url.scheme()!="https" && !(allow_insecure_http && url.scheme()=="http") {return false;}
        let Some(host)=url.host_str().map(str::to_ascii_lowercase) else {return false};
        self.exact.contains(&host) || self.parents.iter().any(|parent|host.strip_suffix(parent.as_str()).is_some_and(|rest|rest.ends_with('.')))
    }
}
/// The ONNX Runtime library shipped beside this executable (release builds place it there), or
/// named by `BEANS_ONNXRUNTIME` for development and tests. A candidate, not a trust decision:
/// installs hash it and show it in their preview like any other file.
pub fn bundled_runtime()->Option<PathBuf> {
    if let Some(path)=std::env::var_os("BEANS_ONNXRUNTIME").map(PathBuf::from) {return path.is_file().then_some(path);}
    let path=std::env::current_exe().ok()?.parent()?.join(AssetKind::Runtime.filename());
    path.is_file().then_some(path)
}
/// The largest archive a runtime is unpacked from.
const MAX_ARCHIVE_BYTES:u64=256*1024*1024;
/// The most redirects one download follows.
const MAX_REDIRECTS:usize=3;
/// The kinds a runtime-backed ONNX embedding installs.
pub const ONNX_ASSETS:[AssetKind;3]=[AssetKind::Runtime,AssetKind::Model,AssetKind::Tokenizer];
struct Pending {plan:AssetPlan,digest:String,expires:Instant}
struct StagingGuard(Option<PathBuf>);
impl Drop for StagingGuard {
    fn drop(&mut self) {
        if let Some(path)=self.0.take() {
            if let Ok(runtime)=tokio::runtime::Handle::try_current() {
                runtime.spawn_blocking(move||{let _=std::fs::remove_dir_all(path);});
            } else {let _=std::fs::remove_dir_all(path);}
        }
    }
}
/// One installer per current account. Core invalidates it on unpair and fences profile revisions.
pub struct AssetInstaller {
    root:PathBuf,
    allowed_origins:BTreeSet<String>,
    allow_insecure_http:bool,
    client:reqwest::Client,
    pending:Mutex<HashMap<String,Pending>>,
}
impl AssetInstaller {
    pub fn new(root:PathBuf,allowed_origins:BTreeSet<String>,allow_insecure_http:bool)->Result<Self,EmbeddingError> {
        Self::with_redirects(root,allowed_origins,allow_insecure_http,RedirectHosts::default())
    }
    /// As `new`, following at most three redirects, each only to `redirects`.
    pub fn with_redirects(root:PathBuf,allowed_origins:BTreeSet<String>,allow_insecure_http:bool,redirects:RedirectHosts)->Result<Self,EmbeddingError> {
        if !root.is_absolute() {return Err(EmbeddingError::InvalidInput);}
        for origin in &allowed_origins {
            let url=super::types::validate_url(origin)?;
            if url.origin().ascii_serialization()!=*origin || (url.scheme()!="https" && !allow_insecure_http) {
                return Err(EmbeddingError::InvalidProfile);
            }
        }
        let policy=if redirects==RedirectHosts::default() {reqwest::redirect::Policy::none()} else {
            reqwest::redirect::Policy::custom(move|attempt| {
                if attempt.previous().len()>MAX_REDIRECTS || !redirects.allows(attempt.url(),allow_insecure_http) {attempt.stop()} else {attempt.follow()}
            })
        };
        let client=beans_tls::client_builder().redirect(policy)
            .connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(300))
            .no_gzip().build().map_err(|_|EmbeddingError::Transport)?;
        Ok(Self{root,allowed_origins,allow_insecure_http,client,pending:Mutex::new(HashMap::new())})
    }
    /// A runtime-backed ONNX embedding: exactly a runtime, a model, and a tokenizer.
    pub fn preview(&self,plan:AssetPlan)->Result<AssetPreview,EmbeddingError> {
        self.preview_assets(plan,&ONNX_ASSETS)
    }
    /// A plan of exactly the `required` kinds, one asset each.
    pub fn preview_assets(&self,plan:AssetPlan,required:&[AssetKind])->Result<AssetPreview,EmbeddingError> {
        self.validate_plan(&plan,required)?;
        let digest=format!("{:x}",Sha256::digest(serde_json::to_vec(&plan).map_err(|_|EmbeddingError::InvalidInput)?));
        let token=uuid::Uuid::new_v4().to_string();
        let preview=AssetPreview {token:token.clone(),digest:digest.clone(),expires_in_seconds:600,
            total_bytes:plan.assets.iter().map(|a|a.bytes).sum(),assets:plan.assets.clone()};
        let mut pending=self.pending.lock();
        pending.retain(|_,p|p.expires>Instant::now());
        if pending.len()>=32 {return Err(EmbeddingError::InvalidInput);}
        pending.insert(token,Pending{plan,digest,expires:Instant::now()+Duration::from_secs(600)});
        Ok(preview)
    }
    fn validate_plan(&self,plan:&AssetPlan,required:&[AssetKind])->Result<(),EmbeddingError> {
        let kinds=plan.assets.iter().map(|a|a.kind).collect::<BTreeSet<_>>();
        let required=required.iter().copied().collect::<BTreeSet<_>>();
        if plan.assets.len()!=required.len() || kinds!=required {return Err(EmbeddingError::InvalidInput);}
        for a in &plan.assets {
            EmbeddingFingerprint::parse(&a.sha256)?;
            if a.bytes==0 || a.bytes>2*1024*1024*1024 || a.license.trim().is_empty() || a.license.len()>4096 {
                return Err(EmbeddingError::InvalidInput);
            }
            match &a.source {
                AssetSource::Supplied{path} if !path.is_absolute()=>return Err(EmbeddingError::InvalidInput),
                AssetSource::Supplied{..}=>{},
                AssetSource::Download{url}=>self.check_download(url)?,
                AssetSource::Archive{url,bytes,sha256,member}=>{
                    self.check_download(url)?;
                    EmbeddingFingerprint::parse(sha256)?;
                    if *bytes==0 || *bytes>MAX_ARCHIVE_BYTES || member.is_empty() || member.len()>512 {return Err(EmbeddingError::InvalidInput);}
                }
            }
        }
        Ok(())
    }
    fn check_download(&self,url:&str)->Result<(),EmbeddingError> {
        let url=super::types::validate_url(url)?;
        if (url.scheme()!="https" && !self.allow_insecure_http)
            || !self.allowed_origins.contains(&url.origin().ascii_serialization()) {return Err(EmbeddingError::InvalidProfile);}
        Ok(())
    }
    pub async fn apply(&self,token:&str,digest:&str,confirmed:bool,cancel:CancellationToken)->Result<LocalBinding,EmbeddingError> {
        let installed=self.apply_assets(token,digest,confirmed,cancel).await?;
        let bound=|kind|installed.get(kind).cloned().ok_or(EmbeddingError::InvalidInput);
        Ok(LocalBinding{runtime:bound(AssetKind::Runtime)?,model:bound(AssetKind::Model)?,tokenizer:bound(AssetKind::Tokenizer)?})
    }
    /// Installs an approved preview of any kinds, as `apply` does.
    pub async fn apply_assets(&self,token:&str,digest:&str,confirmed:bool,cancel:CancellationToken)->Result<InstalledAssets,EmbeddingError> {
        if !confirmed {return Err(EmbeddingError::ApprovalRequired);}
        let pending={
            let mut map=self.pending.lock();
            let p=map.get(token).ok_or(EmbeddingError::ApprovalRequired)?;
            if p.expires<=Instant::now() || p.digest!=digest {return Err(EmbeddingError::ApprovalRequired);}
            map.remove(token).ok_or(EmbeddingError::ApprovalRequired)?
        };
        // The token is consumed before any I/O; failed/cancelled attempts require a new preview.
        if cancel.is_cancelled() {return Err(EmbeddingError::Cancelled);}
        fs::create_dir_all(&self.root).await.map_err(|_|EmbeddingError::InstallFailed)?;
        let root=fs::canonicalize(&self.root).await.map_err(|_|EmbeddingError::InstallFailed)?;
        let id=uuid::Uuid::new_v4().to_string();
        let staging=root.join(format!(".staging-{id}"));let destination=root.join(id);
        fs::create_dir(&staging).await.map_err(|_|EmbeddingError::InstallFailed)?;
        let mut cleanup=StagingGuard(Some(staging.clone()));
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&staging,std::fs::Permissions::from_mode(0o700)).await.map_err(|_|EmbeddingError::InstallFailed)?;
        }
        let result=tokio::select! {biased;
            _=cancel.cancelled()=>Err(EmbeddingError::Cancelled),
            r=tokio::time::timeout(Duration::from_secs(600),self.transfer(&pending.plan,&staging))=>r.map_err(|_|EmbeddingError::Timeout).and_then(|r|r),
        };
        #[cfg(unix)] if result.is_ok() {
            let directory=staging.clone();
            tokio::task::spawn_blocking(move||std::fs::File::open(directory).and_then(|f|f.sync_all()))
                .await.map_err(|_|EmbeddingError::InstallFailed)?.map_err(|_|EmbeddingError::InstallFailed)?;
        }
        if let Err(e)=result {let _=fs::remove_dir_all(&staging).await;return Err(e);}
        if cancel.is_cancelled() {let _=fs::remove_dir_all(&staging).await;return Err(EmbeddingError::Cancelled);}
        if fs::rename(&staging,&destination).await.is_err() {let _=fs::remove_dir_all(&staging).await;return Err(EmbeddingError::InstallFailed);}
        cleanup.0=None;
        // Directory rename is the commit point. Never return a cancelled/failed status after commit.
        #[cfg(unix)] {
            let root_copy=root.clone();
            let _=tokio::task::spawn_blocking(move||std::fs::File::open(root_copy).and_then(|f|f.sync_all())).await;
        }
        Ok(InstalledAssets{assets:pending.plan.assets.iter()
            .map(|a|(a.kind,BoundAsset{path:destination.join(a.kind.filename()),bytes:a.bytes,sha256:a.sha256.clone()})).collect()})
    }
    async fn transfer(&self,plan:&AssetPlan,staging:&Path)->Result<(),EmbeddingError> {
        for a in &plan.assets {
            let path=staging.join(a.kind.filename());
            let mut options=fs::OpenOptions::new();options.write(true).create_new(true);
            #[cfg(unix)] { options.mode(0o600); }
            let mut output=options.open(path).await.map_err(|_|EmbeddingError::InstallFailed)?;
            let mut hash=Sha256::new();let mut size=0u64;
            match &a.source {
                AssetSource::Supplied{path}=>{
                    let mut input=fs::File::open(path).await.map_err(|_|EmbeddingError::SetupRequired)?;
                    let metadata=input.metadata().await.map_err(|_|EmbeddingError::SetupRequired)?;
                    if !metadata.is_file() || metadata.len()!=a.bytes {return Err(EmbeddingError::AssetMismatch);}
                    let mut buffer=vec![0u8;65536];
                    loop {let n=input.read(&mut buffer).await.map_err(|_|EmbeddingError::InstallFailed)?;
                        if n==0 {break;} write_chunk(&mut output,&mut hash,&mut size,&buffer[..n],a.bytes).await?;}
                },
                AssetSource::Download{url}=>{
                    let mut response=self.get(url,a.bytes).await?;
                    while let Some(chunk)=response.chunk().await.map_err(super::api::transport_error)? {
                        write_chunk(&mut output,&mut hash,&mut size,&chunk,a.bytes).await?;
                    }
                }
                AssetSource::Archive{url,bytes,sha256,member}=>{
                    // The whole archive is checked against its pin before anything is read from it.
                    let mut response=self.get(url,*bytes).await?;
                    let (mut archive,mut archive_hash,mut archive_size)=(Vec::new(),Sha256::new(),0u64);
                    while let Some(chunk)=response.chunk().await.map_err(super::api::transport_error)? {
                        if chunk.len() as u64>bytes.saturating_sub(archive_size) {return Err(EmbeddingError::AssetMismatch);}
                        archive_hash.update(&chunk);archive_size+=chunk.len() as u64;archive.extend_from_slice(&chunk);
                    }
                    if archive_size!=*bytes || format!("{:x}",archive_hash.finalize())!=*sha256 {return Err(EmbeddingError::AssetMismatch);}
                    let (member,expected)=(member.clone(),a.bytes);
                    let library=tokio::task::spawn_blocking(move||super::archive::extract_member(&archive,&member,expected))
                        .await.map_err(|_|EmbeddingError::InstallFailed)?.map_err(|error|{tracing::warn!(%error,"unpacking a runtime archive");EmbeddingError::AssetMismatch})?;
                    write_chunk(&mut output,&mut hash,&mut size,&library,a.bytes).await?;
                }
            }
            if size!=a.bytes || format!("{:x}",hash.finalize())!=a.sha256 {return Err(EmbeddingError::AssetMismatch);}
            output.flush().await.map_err(|_|EmbeddingError::InstallFailed)?;
            output.sync_all().await.map_err(|_|EmbeddingError::InstallFailed)?;
        }
        Ok(())
    }
}
impl AssetInstaller {
    /// An approved download's answer, refused unless it is the file itself at the pinned size.
    async fn get(&self,url:&str,bytes:u64)->Result<reqwest::Response,EmbeddingError> {
        let response=self.client.get(url).header(reqwest::header::ACCEPT_ENCODING,"identity").send().await.map_err(super::api::transport_error)?;
        if response.status().is_redirection() {return Err(EmbeddingError::Redirect);}
        if !response.status().is_success() {return Err(EmbeddingError::Service);}
        if response.content_length().is_some_and(|n|n!=bytes) {return Err(EmbeddingError::AssetMismatch);}
        Ok(response)
    }
}
async fn write_chunk(output:&mut fs::File,hash:&mut Sha256,size:&mut u64,chunk:&[u8],expected:u64)->Result<(),EmbeddingError> {
    if chunk.len() as u64>expected.saturating_sub(*size) {return Err(EmbeddingError::AssetMismatch);}
    output.write_all(chunk).await.map_err(|_|EmbeddingError::InstallFailed)?;hash.update(chunk);*size+=chunk.len() as u64;Ok(())
}
/// A fresh private, read-only copy. Native loading never reopens the caller's source path.
#[cfg(feature="embedding-local")]
pub(crate) struct ProtectedRuntime {pub original:BoundAsset,pub path:PathBuf,directory:PathBuf}
#[cfg(feature="embedding-local")]
impl ProtectedRuntime {
    pub(crate) fn prepare(asset:BoundAsset)->Result<Self,EmbeddingError> {
        use std::io::{Read,Write};
        EmbeddingFingerprint::parse(&asset.sha256)?;
        if !asset.path.is_absolute() || asset.bytes==0 || asset.bytes>256*1024*1024 {return Err(EmbeddingError::AssetMismatch);}
        // Keep this single opened handle until every copied byte has been hashed.
        let mut source=std::fs::File::open(&asset.path).map_err(|_|EmbeddingError::SetupRequired)?;
        let metadata=source.metadata().map_err(|_|EmbeddingError::SetupRequired)?;
        if !metadata.is_file() || metadata.len()!=asset.bytes {return Err(EmbeddingError::AssetMismatch);}
        let directory=std::env::temp_dir().join(format!("beans-ort-{}",uuid::Uuid::new_v4()));
        let mut builder=std::fs::DirBuilder::new();
        #[cfg(unix)] {use std::os::unix::fs::DirBuilderExt;builder.mode(0o700);}
        builder.create(&directory).map_err(|_|EmbeddingError::InstallFailed)?;
        let protected=Self{original:asset,path:directory.join(AssetKind::Runtime.filename()),directory};
        let mut options=std::fs::OpenOptions::new();options.write(true).create_new(true);
        #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
        let mut destination=options.open(&protected.path).map_err(|_|EmbeddingError::InstallFailed)?;
        let mut size=0u64;let mut hash=Sha256::new();let mut buffer=[0u8;65536];
        loop {
            let n=source.read(&mut buffer).map_err(|_|EmbeddingError::SetupRequired)?;if n==0 {break;}
            size+=n as u64;if size>protected.original.bytes {return Err(EmbeddingError::AssetMismatch);}
            destination.write_all(&buffer[..n]).map_err(|_|EmbeddingError::InstallFailed)?;
            hash.update(&buffer[..n]);
        }
        if size!=protected.original.bytes || format!("{:x}",hash.finalize())!=protected.original.sha256 {return Err(EmbeddingError::AssetMismatch);}
        destination.sync_all().map_err(|_|EmbeddingError::InstallFailed)?;
        drop(destination);
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&protected.path,std::fs::Permissions::from_mode(0o400)).map_err(|_|EmbeddingError::InstallFailed)?;
            std::fs::set_permissions(&protected.directory,std::fs::Permissions::from_mode(0o500)).map_err(|_|EmbeddingError::InstallFailed)?;
        }
        #[cfg(not(unix))] {
            let mut permissions=std::fs::metadata(&protected.path).map_err(|_|EmbeddingError::InstallFailed)?.permissions();
            permissions.set_readonly(true);std::fs::set_permissions(&protected.path,permissions).map_err(|_|EmbeddingError::InstallFailed)?;
        }
        Ok(protected)
    }
}
#[cfg(feature="embedding-local")]
impl Drop for ProtectedRuntime {
    fn drop(&mut self) {
        #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;
            let _=std::fs::set_permissions(&self.directory,std::fs::Permissions::from_mode(0o700));}
        #[cfg(not(unix))] if let Ok(metadata)=std::fs::metadata(&self.path) {
            let mut permissions=metadata.permissions();permissions.set_readonly(false);
            let _=std::fs::set_permissions(&self.path,permissions);
        }
        let _=std::fs::remove_dir_all(&self.directory);
    }
}
/// Verify supplied/installed bytes again before native load; never trust a saved ready flag.
pub(crate) async fn verified_bytes(asset:&BoundAsset,max:u64)->Result<Vec<u8>,EmbeddingError> {
    EmbeddingFingerprint::parse(&asset.sha256)?;
    if !asset.path.is_absolute() || asset.bytes==0 || asset.bytes>max {return Err(EmbeddingError::AssetMismatch);}
    let mut input=fs::File::open(&asset.path).await.map_err(|_|EmbeddingError::SetupRequired)?;
    let metadata=input.metadata().await.map_err(|_|EmbeddingError::SetupRequired)?;
    if !metadata.is_file() || metadata.len()!=asset.bytes {return Err(EmbeddingError::AssetMismatch);}
    let mut bytes=Vec::with_capacity(asset.bytes as usize);
    (&mut input).take(asset.bytes+1).read_to_end(&mut bytes).await.map_err(|_|EmbeddingError::SetupRequired)?;
    if bytes.len() as u64!=asset.bytes || format!("{:x}",Sha256::digest(&bytes))!=asset.sha256 {return Err(EmbeddingError::AssetMismatch);}
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root()->PathBuf {std::env::temp_dir().join(format!("beans-embedding-assets-{}",uuid::Uuid::new_v4()))}
    async fn supplied()->(PathBuf,AssetPlan) {
        let root=root();fs::create_dir(&root).await.unwrap();let mut assets=Vec::new();
        for kind in [AssetKind::Runtime,AssetKind::Model,AssetKind::Tokenizer] {
            let path=root.join(kind.filename());let bytes=format!("fixture {kind:?}").into_bytes();fs::write(&path,&bytes).await.unwrap();
            assets.push(AssetRequest{kind,source:AssetSource::Supplied{path},license:"fixture CC0".into(),bytes:bytes.len() as u64,sha256:format!("{:x}",Sha256::digest(bytes))});
        }(root,AssetPlan{assets})
    }
    #[tokio::test]
    async fn exact_supplied_preview_consent_install_and_single_use() {
        let (source,plan)=supplied().await;let destination=root();
        let installer=AssetInstaller::new(destination.clone(),BTreeSet::new(),false).unwrap();
        let preview=installer.preview(plan).unwrap();assert!(!destination.exists());
        assert_eq!(installer.apply(&preview.token,&preview.digest,false,CancellationToken::new()).await.unwrap_err(),EmbeddingError::ApprovalRequired);
        assert_eq!(installer.apply(&preview.token,"changed",true,CancellationToken::new()).await.unwrap_err(),EmbeddingError::ApprovalRequired);
        let binding=installer.apply(&preview.token,&preview.digest,true,CancellationToken::new()).await.unwrap();
        assert_eq!(verified_bytes(&binding.model,1024).await.unwrap(),b"fixture Model");
        assert_eq!(installer.apply(&preview.token,&preview.digest,true,CancellationToken::new()).await.unwrap_err(),EmbeddingError::ApprovalRequired);
        fs::remove_dir_all(source).await.unwrap();fs::remove_dir_all(destination).await.unwrap();
    }
    #[tokio::test]
    async fn changed_supplied_file_fails_hash_and_cleans_staging() {
        let (source,plan)=supplied().await;let destination=root();let path=source.join(AssetKind::Model.filename());
        let installer=AssetInstaller::new(destination.clone(),BTreeSet::new(),false).unwrap();let preview=installer.preview(plan).unwrap();
        fs::write(path,b"fixture WRONG").await.unwrap();
        assert_eq!(installer.apply(&preview.token,&preview.digest,true,CancellationToken::new()).await.unwrap_err(),EmbeddingError::AssetMismatch);
        let mut entries=fs::read_dir(&destination).await.unwrap();assert!(entries.next_entry().await.unwrap().is_none());
        fs::remove_dir_all(source).await.unwrap();fs::remove_dir_all(destination).await.unwrap();
    }
    #[tokio::test]
    async fn expired_and_cancelled_approval_never_acquires_files() {
        let (source,plan)=supplied().await;let destination=root();let installer=AssetInstaller::new(destination.clone(),BTreeSet::new(),false).unwrap();
        let preview=installer.preview(plan.clone()).unwrap();installer.pending.lock().get_mut(&preview.token).unwrap().expires=Instant::now();
        assert_eq!(installer.apply(&preview.token,&preview.digest,true,CancellationToken::new()).await.unwrap_err(),EmbeddingError::ApprovalRequired);
        let preview=installer.preview(plan).unwrap();let cancel=CancellationToken::new();cancel.cancel();
        assert_eq!(installer.apply(&preview.token,&preview.digest,true,cancel).await.unwrap_err(),EmbeddingError::Cancelled);
        assert!(!destination.exists());fs::remove_dir_all(source).await.unwrap();
    }
    #[tokio::test]
    async fn exact_origin_allowlist_and_real_fixture_download_with_no_redirects() {
        use tokio::{net::TcpListener,io::{AsyncReadExt,AsyncWriteExt}};
        let (source,mut plan)=supplied().await;let destination=root();
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let origin=format!("http://{}",listener.local_addr().unwrap());
        let runtime=plan.assets.iter_mut().find(|a|a.kind==AssetKind::Runtime).unwrap();runtime.source=AssetSource::Download{url:format!("{origin}/runtime")};
        let denied=AssetInstaller::new(destination.clone(),BTreeSet::new(),true).unwrap();assert!(denied.preview(plan.clone()).is_err());
        let installer=AssetInstaller::new(destination.clone(),[origin].into_iter().collect(),true).unwrap();
        let task=tokio::spawn(async move {let (mut stream,_)=listener.accept().await.unwrap();let mut buf=[0u8;4096];assert!(stream.read(&mut buf).await.unwrap() > 0, "the request arrived");
            stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 15\r\nconnection: close\r\n\r\nfixture Runtime").await.unwrap();});
        let preview=installer.preview(plan).unwrap();let binding=installer.apply(&preview.token,&preview.digest,true,CancellationToken::new()).await.unwrap();task.await.unwrap();
        assert_eq!(verified_bytes(&binding.runtime,1024).await.unwrap(),b"fixture Runtime");
        fs::remove_dir_all(source).await.unwrap();fs::remove_dir_all(destination).await.unwrap();
    }
    #[tokio::test]
    async fn approved_download_rejects_redirect_hash_mismatch_and_stream_overflow() {
        use tokio::{net::TcpListener,io::{AsyncReadExt,AsyncWriteExt}};
        for (response,expected) in [
            ("HTTP/1.1 307 Temporary Redirect\r\nlocation: https://unapproved.invalid/runtime\r\ncontent-length: 0\r\n\r\n",EmbeddingError::Redirect),
            ("HTTP/1.1 200 OK\r\ncontent-length: 15\r\n\r\nfixture CORRUPT",EmbeddingError::AssetMismatch),
            ("HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n10\r\n0123456789abcdef\r\n0\r\n\r\n",EmbeddingError::AssetMismatch),
        ] {
            let (source,mut plan)=supplied().await;let destination=root();
            let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let origin=format!("http://{}",listener.local_addr().unwrap());
            plan.assets.iter_mut().find(|a|a.kind==AssetKind::Runtime).unwrap().source=AssetSource::Download{url:format!("{origin}/runtime")};
            let installer=AssetInstaller::new(destination.clone(),[origin].into_iter().collect(),true).unwrap();
            let preview=installer.preview(plan).unwrap();
            let task=tokio::spawn(async move {let (mut stream,_)=listener.accept().await.unwrap();let mut buf=[0u8;4096];
                assert!(stream.read(&mut buf).await.unwrap() > 0, "the request arrived");stream.write_all(response.as_bytes()).await.unwrap();});
            assert_eq!(installer.apply(&preview.token,&preview.digest,true,CancellationToken::new()).await.unwrap_err(),expected);
            task.await.unwrap();let mut entries=fs::read_dir(&destination).await.unwrap();assert!(entries.next_entry().await.unwrap().is_none());
            fs::remove_dir_all(source).await.unwrap();fs::remove_dir_all(destination).await.unwrap();
        }
    }
    /// A static embedding needs no runtime: exactly its weights and tokenizer. A runtime-backed
    /// plan stays exactly a runtime, a model, and a tokenizer, so a plan with weights in place of
    /// the runtime cannot reach `apply`'s binding.
    #[tokio::test]
    async fn plans_are_exactly_the_kinds_they_install() {
        let source=root();fs::create_dir(&source).await.unwrap();
        let asset=|kind:AssetKind,bytes:&[u8]|{let path=source.join(kind.filename());std::fs::write(&path,bytes).unwrap();
            AssetRequest{kind,source:AssetSource::Supplied{path},license:"fixture CC0".into(),bytes:bytes.len() as u64,sha256:format!("{:x}",Sha256::digest(bytes))}};
        let weights=asset(AssetKind::Weights,b"fixture weights");let tokenizer=asset(AssetKind::Tokenizer,b"fixture tokenizer");
        let model=asset(AssetKind::Model,b"fixture model");
        let destination=root();let installer=AssetInstaller::new(destination.clone(),BTreeSet::new(),false).unwrap();
        let static_plan=AssetPlan{assets:vec![weights.clone(),tokenizer.clone()]};
        assert_eq!(installer.preview(static_plan.clone()).unwrap_err(),EmbeddingError::InvalidInput);
        assert_eq!(installer.preview(AssetPlan{assets:vec![weights.clone(),tokenizer.clone(),model.clone()]}).unwrap_err(),EmbeddingError::InvalidInput);
        assert_eq!(installer.preview_assets(AssetPlan{assets:vec![weights.clone(),weights.clone()]},&[AssetKind::Weights,AssetKind::Tokenizer]).unwrap_err(),EmbeddingError::InvalidInput);
        let preview=installer.preview_assets(static_plan,&[AssetKind::Weights,AssetKind::Tokenizer]).unwrap();
        let installed=installer.apply_assets(&preview.token,&preview.digest,true,CancellationToken::new()).await.unwrap();
        assert_eq!(verified_bytes(installed.get(AssetKind::Weights).unwrap(),1024).await.unwrap(),b"fixture weights");
        assert_eq!(verified_bytes(installed.get(AssetKind::Tokenizer).unwrap(),1024).await.unwrap(),b"fixture tokenizer");
        assert!(installed.get(AssetKind::Runtime).is_none());
        fs::remove_dir_all(source).await.unwrap();fs::remove_dir_all(destination).await.unwrap();
    }
    /// Redirects are followed only to the hosts an installer names, and the pinned size and
    /// checksum still decide; any other redirect fails with nothing installed.
    #[tokio::test]
    async fn downloads_follow_redirects_only_to_named_hosts() {
        use tokio::{net::TcpListener,io::{AsyncReadExt,AsyncWriteExt}};
        let body=b"fixture Weights";
        let serve=|listener:TcpListener,response:String|tokio::spawn(async move {
            let (mut stream,_)=listener.accept().await.unwrap();let mut buf=[0u8;4096];assert!(stream.read(&mut buf).await.unwrap()>0);
            stream.write_all(response.as_bytes()).await.unwrap();});
        for (target_host,followed) in [("localhost",true),("127.0.0.1",false)] {
            let origin_listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let origin=format!("http://{}",origin_listener.local_addr().unwrap());
            let cdn=TcpListener::bind("127.0.0.1:0").await.unwrap();let cdn_port=cdn.local_addr().unwrap().port();
            let location=format!("http://{target_host}:{cdn_port}/blob");
            let first=serve(origin_listener,format!("HTTP/1.1 302 Found\r\nlocation: {location}\r\ncontent-length: 0\r\n\r\n"));
            let second=followed.then(||serve(cdn,format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\nfixture Weights",body.len())));
            let destination=root();
            let redirects=RedirectHosts{exact:["localhost".to_string()].into(),parents:BTreeSet::new()};
            let installer=AssetInstaller::with_redirects(destination.clone(),[origin.clone()].into(),true,redirects).unwrap();
            let plan=AssetPlan{assets:vec![AssetRequest{kind:AssetKind::Weights,source:AssetSource::Download{url:format!("{origin}/resolve/main/model.safetensors")},
                license:"fixture CC0".into(),bytes:body.len() as u64,sha256:format!("{:x}",Sha256::digest(body))}]};
            let preview=installer.preview_assets(plan,&[AssetKind::Weights]).unwrap();
            let result=installer.apply_assets(&preview.token,&preview.digest,true,CancellationToken::new()).await;
            first.await.unwrap();
            if let Some(second)=second {second.await.unwrap();
                assert_eq!(verified_bytes(result.unwrap().get(AssetKind::Weights).unwrap(),1024).await.unwrap(),body);
            } else {
                assert_eq!(result.unwrap_err(),EmbeddingError::Redirect);
                let mut entries=fs::read_dir(&destination).await.unwrap();assert!(entries.next_entry().await.unwrap().is_none());
            }
            fs::remove_dir_all(destination).await.unwrap();
        }
        let hosts=RedirectHosts{exact:["huggingface.co".to_string()].into(),parents:["hf.co".to_string()].into()};
        for (url,allowed) in [("https://huggingface.co/x",true),("https://cdn-lfs.hf.co/x",true),("https://cas-bridge.xethub.hf.co/x",true),
            ("https://evilhf.co/x",false),("https://hf.co.evil.com/x",false),("http://cdn-lfs.hf.co/x",false),("https://huggingface.co.evil/x",false)] {
            assert_eq!(hosts.allows(&reqwest::Url::parse(url).unwrap(),false),allowed,"{url}");
        }
    }
    #[cfg(feature="embedding-local")]
    #[tokio::test]
    async fn protected_runtime_bytes_survive_source_replacement_and_reject_wrong_checksum() {
        let (source,plan)=supplied().await;
        let request=plan.assets.iter().find(|a|a.kind==AssetKind::Runtime).unwrap();
        let AssetSource::Supplied{path}=&request.source else {panic!("supplied fixture");};
        let approved=BoundAsset{path:path.clone(),bytes:request.bytes,sha256:request.sha256.clone()};
        let protected=ProtectedRuntime::prepare(approved.clone()).unwrap();
        assert_ne!(protected.path,approved.path);
        fs::write(&approved.path,b"unapproved data").await.unwrap();
        assert_eq!(fs::read(&protected.path).await.unwrap(),b"fixture Runtime");
        assert!(matches!(ProtectedRuntime::prepare(approved),Err(EmbeddingError::AssetMismatch)));
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&protected.directory).unwrap().permissions().mode()&0o777,0o500);
            assert_eq!(std::fs::metadata(&protected.path).unwrap().permissions().mode()&0o777,0o400);
        }
        let protected_path=protected.path.clone();drop(protected);
        assert!(!protected_path.exists());fs::remove_dir_all(source).await.unwrap();
    }
    #[cfg(all(feature="embedding-local",unix))]
    #[tokio::test]
    async fn replacing_supplied_symlink_cannot_redirect_protected_load_path() {
        let (source,plan)=supplied().await;
        let request=plan.assets.iter().find(|a|a.kind==AssetKind::Runtime).unwrap();
        let AssetSource::Supplied{path}=&request.source else {panic!("supplied fixture");};
        let link=source.join("selected-runtime");
        std::os::unix::fs::symlink(path,&link).unwrap();
        let protected=ProtectedRuntime::prepare(BoundAsset{path:link.clone(),bytes:request.bytes,sha256:request.sha256.clone()}).unwrap();
        let wrong=source.join("wrong-runtime");fs::write(&wrong,b"unapproved data").await.unwrap();
        fs::remove_file(&link).await.unwrap();std::os::unix::fs::symlink(wrong,&link).unwrap();
        assert!(!std::fs::symlink_metadata(&protected.path).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(&protected.path).await.unwrap(),b"fixture Runtime");
        drop(protected);fs::remove_dir_all(source).await.unwrap();
    }
}
