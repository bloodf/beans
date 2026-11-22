//! A Runner's side of local schedule models: installing one the user approved, choosing how
//! messages are read, and the read itself.
//!
//! Installs follow local memory setup: a preview freezes exactly which files, from where, of
//! what size and SHA-256; applying it needs that preview's one-use token and digest, the user's
//! confirmation, the same account, and a requester still paired. Nothing downloads at startup or
//! as a fallback, and a file the catalog does not pin downloads only after it is pinned.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use super::classifier::Classifier;
use super::{Catalog, Detection, Engine, MAX_MODEL_BYTES};
use crate::app::App;
#[cfg(feature = "embedding-local")]
use crate::runner_state::Loader;
use crate::runner_state::{AccountFile, Settings};
use super::hub::Hub;
use crate::embeddings::assets::{AssetInstaller, AssetKind, AssetPlan, AssetRequest, AssetSource, InstalledAssets};
use crate::embeddings::runtime_download::{self, Pinned};

const ASSET_DIR: &str = "schedule-models/assets";
const APPROVAL_SECS: u64 = 600;
const MAX_APPROVALS: usize = 16;
/// The largest ONNX Runtime library a user may supply, as local memory embeddings allow.
const MAX_RUNTIME_BYTES: u64 = 256 * 1024 * 1024;
const RUNTIME_LICENSE: &str = "ONNX Runtime (MIT)";
/// Above this, a message reads as a request in `local` mode: more likely than not.
pub const LOCAL_THRESHOLD: f32 = 0.0;
/// Above this, `local_then_reader` passes a message on to the small model. Lower than
/// `LOCAL_THRESHOLD`, since a message it stops gets no note at all.
pub const GATE_THRESHOLD: f32 = -1.0;
/// How long a turn waits for the local read.
pub const READ_WAIT: Duration = Duration::from_secs(3);
/// The longest quote a local read hands the turn.
#[cfg(feature = "embedding-local")]
const MAX_QUOTE_CHARS: usize = 300;

/// Where ONNX and image models get their runtime when the user names none: the library beside
/// this executable (the apps ship one), else the pinned wheel for this platform, downloaded as
/// part of the model's approved preview.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RuntimeSupply {
    pub beside: Option<PathBuf>,
    pub download: Option<Pinned>,
}

impl RuntimeSupply {
    fn this() -> RuntimeSupply {
        RuntimeSupply { beside: crate::embeddings::assets::bundled_runtime(), download: runtime_download::pinned() }
    }

    /// Disk an ONNX model's install takes for its runtime: its library, when it downloads one.
    pub(crate) fn download_bytes(&self) -> u64 {
        if self.beside.is_some() {
            return 0;
        }
        self.download.as_ref().map_or(0, |pin| pin.library_bytes)
    }

    fn origin(&self) -> Option<String> {
        self.download.as_ref().and_then(|pin| reqwest::Url::parse(&pin.url).ok()).map(|url| url.origin().ascii_serialization())
    }
}

/// What the Runner keeps between runs, for one account.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Saved {
    mode: Detection,
    selected: Option<String>,
    installs: BTreeMap<String, Install>,
}

impl Settings for Saved {
    const PATH: &'static str = "schedule-models/state.json";
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Install {
    assets: InstalledAssets,
    /// Trained for these files on `training.json`, once.
    classifier: Option<Classifier>,
    installed_at: f64,
}

impl Install {
    /// What identifies these exact files.
    fn fingerprint(&self) -> String {
        self.assets.assets.iter().map(|(_, a)| a.sha256.as_str()).collect::<Vec<_>>().join(":")
    }
}

struct Approval {
    account: String,
    actor: String,
    expires: Instant,
    model_id: String,
    installer: Arc<AssetInstaller>,
    token: String,
    digest: String,
}


/// A model ready to read messages.
#[cfg(feature = "embedding-local")]
struct Loaded {
    encoder: super::engine::Encoder,
    classifier: Classifier,
}

/// One install in progress (#581).
struct Installing {
    model_id: String,
    progress: Arc<crate::embeddings::assets::InstallProgress>,
    cancel: CancellationToken,
}

/// The Runner's schedule-model state in memory.
#[derive(Default)]
pub struct Runtime {
    approvals: Mutex<HashMap<String, Approval>>,
    installer: Mutex<Option<(String, Arc<AssetInstaller>)>>,
    settings: AccountFile<Saved>,
    /// The catalog in place of the built-in one, for tests.
    catalog: Mutex<Option<&'static Catalog>>,
    /// The hub in place of `Hub::from_env`, for tests.
    hub: Mutex<Option<Hub>>,
    /// The computer in place of this one, for tests.
    machine: Mutex<Option<super::recommend::Machine>>,
    /// Where runtimes come from, in place of this build's, for tests.
    runtime_supply: Mutex<Option<RuntimeSupply>>,
    /// Cancels installs when the account pauses or is forgotten.
    cancel: Mutex<CancellationToken>,
    /// The install under way: its model, bytes so far, and the token `schedule_models.cancel`
    /// stops it with.
    installing: Mutex<Option<Installing>>,
    /// The selected model, by `load_key`.
    #[cfg(feature = "embedding-local")]
    models: Arc<Loader<Loaded>>,
}

impl Runtime {
    /// Drops approvals, loaded models, and cached state, and stops installs and loads.
    pub fn clear(&self) {
        let mut cancel = self.cancel.lock();
        cancel.cancel();
        *cancel = CancellationToken::new();
        drop(cancel);
        self.approvals.lock().clear();
        *self.installer.lock() = None;
        self.settings.forget();
        #[cfg(feature = "embedding-local")]
        self.models.reset();
    }

    fn cancel_token(&self) -> CancellationToken {
        self.cancel.lock().child_token()
    }

    fn hub(&self) -> Hub {
        self.hub.lock().clone().unwrap_or_else(Hub::from_env)
    }

    /// This computer, as a suggestion reads it; tests set another.
    fn machine(&self, app: &App) -> super::recommend::Machine {
        self.machine.lock().clone().unwrap_or_else(|| crate::machine::this(&app.config.home.join(ASSET_DIR), &self.runtime_supply()))
    }

    fn runtime_supply(&self) -> RuntimeSupply {
        self.runtime_supply.lock().clone().unwrap_or_else(RuntimeSupply::this)
    }

    #[cfg(test)]
    pub(crate) fn set_runtime_supply(&self, supply: RuntimeSupply) {
        *self.runtime_supply.lock() = Some(supply);
        *self.installer.lock() = None;
    }

    #[cfg(test)]
    pub(crate) fn set_machine(&self, machine: super::recommend::Machine) {
        *self.machine.lock() = Some(machine);
    }

    /// Sets the mode as `schedule_models.select` would, without a model.
    #[cfg(test)]
    pub(crate) fn set_mode(app: &App, mode: Detection) {
        edit(
            app,
            || Ok(()),
            |saved| {
                saved.mode = mode;
                Ok(())
            },
        )
        .unwrap();
    }

    /// Reads `catalog` in place of the built-in one, so tests do not depend on which shipped
    /// models are pinned yet.
    #[cfg(test)]
    pub(crate) fn set_catalog(&self, catalog: Catalog) {
        *self.catalog.lock() = Some(Box::leak(Box::new(catalog)));
    }

    #[cfg(test)]
    pub(crate) fn set_hub(&self, hub: Hub) {
        *self.hub.lock() = Some(hub);
        *self.installer.lock() = None;
    }
}

/// The catalog this Runner offers.
fn catalog(app: &App) -> &'static Catalog {
    app.schedule_models.catalog.lock().unwrap_or_else(super::catalog)
}

fn account(app: &App) -> Result<String, String> {
    crate::runner_state::account(app)
}

/// The saved state for the current account; another account's reads as nothing saved.
fn saved(app: &App) -> Result<Saved, String> {
    app.schedule_models.settings.get(app)
}

/// Reads, changes, and writes the saved state as one step, while `still` holds.
fn edit<T>(app: &App, still: impl Fn() -> Result<(), String>, change: impl FnOnce(&mut Saved) -> Result<T, String>) -> Result<T, String> {
    app.schedule_models.settings.edit(app, still, change)
}

/// Who asked, for which account, and whether both still hold: the account is the same, the
/// requester is this Device or still paired, and the account is not paused.
pub(crate) struct Authority {
    actor: String,
    account: String,
    incarnation: u64,
}

impl Authority {
    pub(crate) fn capture(app: &App, actor: &str) -> Result<Authority, String> {
        let incarnation = app.plugin_admission(None).map_err(|_| "authority_changed".to_string())?.get().0;
        let authority = Authority {
            actor: actor.into(),
            account: account(app)?,
            incarnation,
        };
        authority.check(app)?;
        Ok(authority)
    }

    pub(crate) fn check(&self, app: &App) -> Result<(), String> {
        let _lifecycle = app.plugin_admission(Some(self.incarnation)).map_err(|_| "authority_changed".to_string())?;
        if account(app)? != self.account {
            return Err("authority_changed".into());
        }
        let local = app.this_device_id();
        let state = app.state.lock().unwrap();
        if local.as_deref() != Some(self.actor.as_str()) && !state.listed_machines.contains_key(&self.actor) {
            return Err("requester_revoked".into());
        }
        if state.paused {
            return Err("account_paused".into());
        }
        Ok(())
    }
}

fn decode<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|_| "invalid_request".to_string())
}

/// Serves `schedule_models.*` on this Runner for `actor`.
pub async fn serve(app: &Arc<App>, method: &str, params: Value, actor: &str) -> Result<Value, String> {
    let authority = Authority::capture(app, actor)?;
    let _admission = app.update.try_admit().ok_or("runner_draining")?;
    if !app.local_device().is_some_and(|d| d.is_runner()) {
        return Err("runner_required".into());
    }
    match method {
        "schedule_models.list" => list(app),
        "schedule_models.status" => status(app),
        "schedule_models.preview" => preview(app, &authority, params).await,
        "schedule_models.apply" => apply(app, &authority, params).await,
        "schedule_models.select" => select(app, &authority, params).await,
        "schedule_models.remove" => remove(app, &authority, params).await,
        "schedule_models.progress" => Ok(progress(app)),
        "schedule_models.cancel" => cancel_install(app, params),
        _ => Err("unknown_schedule_models_method".into()),
    }
}

/// How many recent messages a suggestion reads for their languages.
const LANGUAGE_SAMPLE: usize = 300;

fn list(app: &App) -> Result<Value, String> {
    let saved = saved(app)?;
    let catalog = catalog(app);
    let hub = app.schedule_models.hub();
    let machine = app.schedule_models.machine(app);
    let (texts, images) = app.store.chat_sample(LANGUAGE_SAMPLE).map_err(|_| "storage_failed")?;
    let needs = super::recommend::Needs { multilingual: super::recommend::needs_multilingual(&texts), images };
    let (fits, recommended) = super::recommend::recommend(catalog, &machine, &needs, |id| saved.installs.contains_key(id));
    let models: Vec<Value> = catalog
        .models
        .iter()
        .zip(fits)
        .map(|(model, (_, fit))| {
            let mut listing = model.listing(&hub);
            listing["installed"] = saved.installs.contains_key(&model.id).into();
            listing["selected"] = (saved.selected.as_deref() == Some(model.id.as_str())).into();
            listing["fit"] = json!(fit);
            listing
        })
        .collect();
    Ok(json!({
        "note": catalog.note,
        "max_bytes": MAX_MODEL_BYTES,
        "models": models,
        "machine": machine,
        "needs": needs,
        "recommended": recommended,
        "mode": saved.mode,
        "selected": saved.selected,
        "runs_models": cfg!(feature = "embedding-local"),
        "onnx_runtime": "ONNX and image models run on the ONNX Runtime shared library (onnxruntime.dll, libonnxruntime.dylib, or libonnxruntime.so), version 1.27 or newer, CPU build: the one beside this CLI, else the pinned official build for this computer, downloaded with the model once you approve its preview, or one whose path you pass as `runtime`. Static models need nothing else.",
    }))
}

fn status(app: &App) -> Result<Value, String> {
    let saved = saved(app)?;
    Ok(json!({
        "mode": saved.mode,
        "selected": saved.selected,
        "installed": saved.installs.keys().collect::<Vec<_>>(),
        "state": load_state(app, &saved),
        "error": load_error(app, &saved),
    }))
}

/// What identifies a loaded model: the account, the model, and its exact installed files.
fn load_key(app: &App, saved: &Saved, model_id: &str) -> Option<String> {
    let install = saved.installs.get(model_id)?;
    Some(format!("{}\0{model_id}\0{}", account(app).ok()?, install.fingerprint()))
}

/// Whether the selected model is ready to read messages.
fn load_state(app: &App, saved: &Saved) -> &'static str {
    let Some(key) = saved.selected.as_deref().and_then(|id| load_key(app, saved, id)) else { return "none_selected" };
    #[cfg(feature = "embedding-local")]
    return app.schedule_models.models.status(&key);
    #[cfg(not(feature = "embedding-local"))]
    {
        let _ = key;
        "runtime_unavailable"
    }
}

fn load_error(app: &App, saved: &Saved) -> Option<String> {
    let key = saved.selected.as_deref().and_then(|id| load_key(app, saved, id))?;
    #[cfg(feature = "embedding-local")]
    return app.schedule_models.models.error(&key);
    #[cfg(not(feature = "embedding-local"))]
    {
        let _ = key;
        None
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Source {
    /// From the catalog's Hugging Face URLs: only for a pinned model.
    Download,
    /// Files the user downloaded, by role (`weights` or `model`, and `tokenizer`), as absolute paths.
    Supplied { files: BTreeMap<String, PathBuf> },
}

async fn preview(app: &Arc<App>, authority: &Authority, params: Value) -> Result<Value, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        model_id: String,
        source: Source,
        /// The ONNX Runtime library, for an ONNX model.
        #[serde(default)]
        runtime: Option<PathBuf>,
    }
    let request: Request = decode(params)?;
    let model = catalog(app).model(&request.model_id).ok_or("unknown_model")?;
    let cancel = app.schedule_models.cancel_token();
    let hub = app.schedule_models.hub();
    let supplied_source = matches!(request.source, Source::Supplied { .. });
    let mut assets = Vec::new();
    if let Source::Supplied { files } = &request.source {
        if files.keys().any(|role| !model.files().iter().any(|(r, _)| r == role)) {
            return Err("unknown_file_role".into());
        }
    }
    for (role, file) in model.files() {
        let kind = kind_of(role);
        let asset = match &request.source {
            Source::Download => {
                let (Some(bytes), Some(sha256)) = (file.bytes, file.sha256.clone()) else {
                    return Err("model_not_pinned".into());
                };
                AssetRequest {
                    kind,
                    source: AssetSource::Download {
                        url: hub.file_url(model, file),
                    },
                    license: model.license.clone(),
                    bytes,
                    sha256,
                }
            }
            Source::Supplied { files } => {
                let path = files.get(role).ok_or("file_required")?;
                let (bytes, sha256) = hash_file(path, MAX_MODEL_BYTES, &cancel).await?;
                if file.bytes.is_some_and(|b| b != bytes) || file.sha256.as_ref().is_some_and(|s| s != &sha256) {
                    return Err("asset_mismatch".into());
                }
                AssetRequest {
                    kind,
                    source: AssetSource::Supplied { path: path.clone() },
                    license: model.license.clone(),
                    bytes,
                    sha256,
                }
            }
        };
        assets.push(asset);
    }
    if assets.iter().map(|a| a.bytes).sum::<u64>() > MAX_MODEL_BYTES {
        return Err("model_too_large".into());
    }
    // ONNX and image models use the runtime the user names, else the one shipped beside this
    // executable, else the pinned one for this platform, downloaded with the model. Whichever it
    // is, the preview shows it, by size and SHA-256, like the model's own files.
    let supply = app.schedule_models.runtime_supply();
    let mut runtime_fetch = None;
    if model.engine == Engine::Static {
        if request.runtime.is_some() {
            return Err("runtime_not_needed".into());
        }
    } else {
        let asset = match (request.runtime.clone().or_else(|| supply.beside.clone()), &supply.download) {
            (Some(path), _) => {
                let (bytes, sha256) = hash_file(&path, MAX_RUNTIME_BYTES, &cancel).await?;
                AssetRequest { kind: AssetKind::Runtime, source: AssetSource::Supplied { path }, license: RUNTIME_LICENSE.into(), bytes, sha256 }
            }
            (None, Some(pin)) => {
                let from = reqwest::Url::parse(&pin.url).ok().and_then(|url| url.host_str().map(str::to_owned)).unwrap_or_default();
                runtime_fetch = Some(json!({ "version": runtime_download::version(), "bytes": pin.bytes, "from": from }));
                AssetRequest {
                    kind: AssetKind::Runtime,
                    source: AssetSource::Archive { url: pin.url.clone(), bytes: pin.bytes, sha256: pin.sha256.clone(), member: pin.member.clone() },
                    license: RUNTIME_LICENSE.into(),
                    bytes: pin.library_bytes,
                    sha256: pin.library_sha256.clone(),
                }
            }
            (None, None) => return Err("runtime_required".into()),
        };
        assets.push(asset);
    }
    let kinds: Vec<AssetKind> = assets.iter().map(|a| a.kind).collect();
    let installer = installer(app, &hub, &supply)?;
    authority.check(app)?;
    let preview = installer.preview_assets(AssetPlan { assets }, &kinds).map_err(|_| "invalid_asset_plan")?;
    let token = uuid::Uuid::new_v4().to_string();
    {
        let mut approvals = app.schedule_models.approvals.lock();
        approvals.retain(|_, a| a.expires > Instant::now());
        if approvals.len() >= MAX_APPROVALS {
            return Err("too_many_approvals".into());
        }
        approvals.insert(
            token.clone(),
            Approval {
                account: authority.account.clone(),
                actor: authority.actor.clone(),
                expires: Instant::now() + Duration::from_secs(APPROVAL_SECS),
                model_id: model.id.clone(),
                installer,
                token: preview.token.clone(),
                digest: preview.digest.clone(),
            },
        );
    }
    Ok(json!({
        "preview_token": token,
        "preview_digest": preview.digest,
        "expires_in_seconds": APPROVAL_SECS,
        "runner_id": app.this_device_id(),
        "model_id": model.id,
        "name": model.name,
        "license": model.license,
        "pinned": model.pinned(),
        "source": if supplied_source { "supplied" } else { "download" },
        // Where the files come from, for the approval to name.
        "hub": (!supplied_source).then(|| hub.label()),
        // The ONNX Runtime downloaded with the model, when none is here: `{version, bytes, from}`.
        "runtime_download": runtime_fetch,
        "total_bytes": preview.total_bytes,
        "assets": preview.assets,
    }))
}

fn kind_of(role: &str) -> AssetKind {
    match role {
        "weights" => AssetKind::Weights,
        "image_model" => AssetKind::ImageModel,
        "model" => AssetKind::Model,
        _ => AssetKind::Tokenizer,
    }
}

/// One installer per account, hub, and runtime source, downloading only from the hub's origin
/// and, for a runtime, the origin of its pinned wheel.
fn installer(app: &App, hub: &Hub, supply: &RuntimeSupply) -> Result<Arc<AssetInstaller>, String> {
    let account = account(app)?;
    let runtime_origin = supply.origin();
    let owner = format!("{account}\0{}\0{}", hub.base, runtime_origin.as_deref().unwrap_or(""));
    let mut installer = app.schedule_models.installer.lock();
    if let Some((_, installer)) = installer.as_ref().filter(|(cached, _)| *cached == owner) {
        return Ok(installer.clone());
    }
    let namespace = format!("{:x}", Sha256::digest(format!("beans-schedule-models\0{account}").as_bytes()));
    let root = app.config.home.join(ASSET_DIR).join(&namespace[..32]);
    let origins = [Some(hub.origin()?), runtime_origin].into_iter().flatten().collect();
    let created = Arc::new(AssetInstaller::with_redirects(root, origins, hub.allow_insecure_http, hub.redirects.clone()).map_err(|_| "invalid_model_hub")?);
    *installer = Some((owner, created.clone()));
    Ok(created)
}

/// The size and SHA-256 of a file the user supplied, read once, refusing one bigger than `max`.
async fn hash_file(path: &Path, max: u64, cancel: &CancellationToken) -> Result<(u64, String), String> {
    if !path.is_absolute() {
        return Err("absolute_path_required".into());
    }
    let mut file = tokio::fs::File::open(path).await.map_err(|_| "file_unavailable")?;
    let metadata = file.metadata().await.map_err(|_| "file_unavailable")?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("file_unavailable".into());
    }
    if metadata.len() > max {
        return Err("model_too_large".into());
    }
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = vec![0u8; 65536];
    loop {
        if cancel.is_cancelled() {
            return Err("cancelled".into());
        }
        let n = file.read(&mut buffer).await.map_err(|_| "file_unavailable")?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size > max {
            return Err("model_too_large".into());
        }
        hash.update(&buffer[..n]);
    }
    Ok((size, format!("{:x}", hash.finalize())))
}

async fn apply(app: &Arc<App>, authority: &Authority, params: Value) -> Result<Value, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        preview_token: String,
        preview_digest: String,
        confirm: bool,
    }
    let request: Request = decode(params)?;
    if !request.confirm {
        return Err("confirmation_required".into());
    }
    // One use: the approval is gone whatever happens next.
    let approval = app.schedule_models.approvals.lock().remove(&request.preview_token).ok_or("approval_not_found")?;
    if approval.expires <= Instant::now() {
        return Err("approval_expired".into());
    }
    if approval.account != authority.account || approval.actor != authority.actor || approval.digest != request.preview_digest {
        return Err("approval_stale".into());
    }
    authority.check(app)?;
    let cancel = app.schedule_models.cancel_token();
    let total = approval.installer.planned_bytes(&approval.token).unwrap_or(0);
    let progress = Arc::new(crate::embeddings::assets::InstallProgress { total, ..Default::default() });
    {
        let mut installing = app.schedule_models.installing.lock();
        if installing.is_some() {
            return Err("install_in_progress".into());
        }
        *installing = Some(Installing { model_id: approval.model_id.clone(), progress: progress.clone(), cancel: cancel.clone() });
    }
    let installed = approval
        .installer
        .apply_assets_with_progress(&approval.token, &approval.digest, true, cancel.clone(), Some(progress.clone()))
        .await;
    app.schedule_models.installing.lock().take_if(|current| Arc::ptr_eq(&current.progress, &progress));
    let installed = installed.map_err(|error| if error == crate::embeddings::EmbeddingError::Cancelled { "install_cancelled" } else { "asset_install_failed" })?;
    let install = Install {
        assets: installed.clone(),
        classifier: None,
        installed_at: crate::config::now_secs(),
    };
    let previous = match edit(app, || authority.check(app), |saved| Ok(saved.installs.insert(approval.model_id.clone(), install))) {
        Ok(previous) => previous,
        Err(error) => {
            remove_files(&installed);
            return Err(error);
        }
    };
    if let Some(previous) = previous {
        remove_files(&previous.assets);
    }
    // An image model is for search; only text models read schedules.
    let image_model = catalog(app).model(&approval.model_id).is_some_and(|m| m.engine == Engine::Clip);
    let state = if image_model { "installed".into() } else { prepare_now(app, &approval.model_id).await };
    Ok(json!({ "installed": true, "model_id": approval.model_id, "state": state }))
}

/// The install under way, for the apps to show: its model and bytes so far and in all.
fn progress(app: &App) -> Value {
    match app.schedule_models.installing.lock().as_ref() {
        Some(installing) => json!({ "installing": { "model_id": installing.model_id, "done_bytes": installing.progress.done(), "total_bytes": installing.progress.total } }),
        None => json!({ "installing": null }),
    }
}

/// Stops the install under way (of `model_id`, when given). Nothing partial is kept.
fn cancel_install(app: &App, params: Value) -> Result<Value, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        #[serde(default)]
        model_id: Option<String>,
    }
    let request: Request = decode(params)?;
    let installing = app.schedule_models.installing.lock();
    let cancelled = match installing.as_ref() {
        Some(current) if request.model_id.as_ref().is_none_or(|id| *id == current.model_id) => {
            current.cancel.cancel();
            true
        }
        _ => false,
    };
    Ok(json!({ "cancelled": cancelled }))
}

/// Deletes an install's directory.
fn remove_files(assets: &InstalledAssets) {
    if let Some(directory) = assets.assets.first().and_then(|(_, a)| a.path.parent()) {
        let _ = std::fs::remove_dir_all(directory);
    }
}

async fn select(app: &Arc<App>, authority: &Authority, params: Value) -> Result<Value, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        mode: Detection,
        #[serde(default)]
        model_id: Option<String>,
    }
    let request: Request = decode(params)?;
    if matches!(request.mode, Detection::Local | Detection::LocalThenReader) && request.model_id.is_none() {
        return Err("model_required".into());
    }
    let saved = edit(
        app,
        || authority.check(app),
        |saved| {
            if request.model_id.as_ref().is_some_and(|id| !saved.installs.contains_key(id)) {
                return Err("model_not_installed".into());
            }
            if request.model_id.as_ref().and_then(|id| catalog(app).model(id)).is_some_and(|m| m.engine == Engine::Clip) {
                return Err("image_model_reads_no_schedules".into());
            }
            saved.mode = request.mode;
            saved.selected = request.model_id.clone();
            Ok(saved.clone())
        },
    )?;
    // A new choice is a fresh start, a failed load included.
    #[cfg(feature = "embedding-local")]
    app.schedule_models.models.reset();
    let state = match &request.model_id {
        Some(id) => prepare_now(app, id).await,
        None => "none_selected".into(),
    };
    Ok(json!({ "mode": saved.mode, "selected": saved.selected, "state": state }))
}

async fn remove(app: &Arc<App>, authority: &Authority, params: Value) -> Result<Value, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        model_id: String,
    }
    let request: Request = decode(params)?;
    let (install, saved) = edit(
        app,
        || authority.check(app),
        |saved| {
            let install = saved.installs.remove(&request.model_id).ok_or("model_not_installed")?;
            if saved.selected.as_deref() == Some(request.model_id.as_str()) {
                // A local mode with nothing selected reads nothing, and spends no tokens.
                saved.selected = None;
            }
            Ok((install, saved.clone()))
        },
    )?;
    #[cfg(feature = "embedding-local")]
    app.schedule_models.models.reset();
    remove_files(&install.assets);
    Ok(json!({ "removed": true, "mode": saved.mode, "selected": saved.selected }))
}

/// Loads `model_id`, training its classifier the first time, and says how that went.
async fn prepare_now(app: &Arc<App>, model_id: &str) -> String {
    #[cfg(feature = "embedding-local")]
    {
        let Some(key) = saved(app).ok().and_then(|saved| load_key(app, &saved, model_id)) else { return "model_not_installed".into() };
        let (app, model_id) = (app.clone(), model_id.to_string());
        match app.schedule_models.models.clone().load_now(&key, |cancel| async move { load(&app, &model_id, cancel).await }).await {
            Ok(_) => "ready".into(),
            Err(error) => error,
        }
    }
    #[cfg(not(feature = "embedding-local"))]
    {
        let _ = (app, model_id);
        "runtime_unavailable".into()
    }
}

#[cfg(feature = "embedding-local")]
async fn load(app: &Arc<App>, model_id: &str, cancel: CancellationToken) -> Result<Loaded, String> {
    use super::classifier::{parts, training_messages};
    let account = account(app)?;
    let saved = saved(app)?;
    let install = saved.installs.get(model_id).cloned().ok_or("model_not_installed")?;
    let model = catalog(app).model(model_id).ok_or("unknown_model")?;
    let encoder = super::engine::Encoder::load(model, &install.assets, cancel.clone()).await.map_err(error_code)?;
    let classifier = match install.classifier.clone().filter(|c| c.fits(model.dimensions())) {
        Some(classifier) => classifier,
        None => {
            let examples = training_messages();
            let texts: Vec<String> = examples.iter().map(|e| e.text.clone()).collect();
            let labels: Vec<bool> = examples.iter().map(|e| e.recurring).collect();
            let vectors = encoder.encode(&texts, &cancel).await.map_err(error_code)?;
            let classifier = tokio::task::spawn_blocking(move || Classifier::train(&vectors, &labels))
                .await
                .map_err(|_| "training_failed")?
                .ok_or("training_failed")?;
            // Kept with the install it was trained for, unless that changed meanwhile.
            edit(
                app,
                || if self::account(app)? == account { Ok(()) } else { Err("authority_changed".into()) },
                |current| {
                    if let Some(kept) = current.installs.get_mut(model_id).filter(|i| i.fingerprint() == install.fingerprint()) {
                        kept.classifier = Some(classifier.clone());
                    }
                    Ok(())
                },
            )?;
            classifier
        }
    };
    // Check the classifier reads the model's own vectors before trusting it.
    let probe = encoder.encode(&parts("every day"), &cancel).await.map_err(error_code)?;
    if classifier.best(&probe).is_none_or(|(_, score)| !score.is_finite()) {
        return Err("training_failed".into());
    }
    Ok(Loaded { encoder, classifier })
}

#[cfg(feature = "embedding-local")]
fn error_code(error: crate::embeddings::EmbeddingError) -> String {
    use crate::embeddings::EmbeddingError as E;
    match error {
        E::SetupRequired => "files_missing",
        E::AssetMismatch => "files_changed",
        E::RuntimeUnavailable | E::RuntimeConflict => "runtime_unavailable",
        E::ModelContract | E::InvalidProfile => "model_contract",
        E::Cancelled => "cancelled",
        E::Timeout => "timeout",
        _ => "load_failed",
    }
    .into()
}

/// An installed catalog model, for other local uses (semantic search): its catalog entry, its
/// checked files, and what identifies those exact files. Installing and removing stay here.
pub(crate) fn installed(app: &App, model_id: &str) -> Result<(super::CatalogModel, InstalledAssets, String), String> {
    let saved = saved(app)?;
    let install = saved.installs.get(model_id).ok_or("model_not_installed")?;
    let model = catalog(app).model(model_id).ok_or("unknown_model")?;
    Ok((model.clone(), install.assets.clone(), install.fingerprint()))
}

/// How this Runner reads messages for a schedule now.
pub fn mode(app: &App) -> Detection {
    saved(app).map(|s| s.mode).unwrap_or_default()
}

/// The selected model's read of `text`: the part most like a request to repeat something, and
/// its score (log-odds). `None` when no model is selected or ready; a model selected but not
/// loaded yet starts loading for the next message.
pub async fn read(app: &Arc<App>, text: &str, cancel: &CancellationToken) -> Option<(String, f32)> {
    #[cfg(feature = "embedding-local")]
    {
        let saved = saved(app).ok()?;
        let selected = saved.selected.clone()?;
        let key = load_key(app, &saved, &selected)?;
        let Some(loaded) = app.schedule_models.models.ready(&key) else {
            // Loaded in the background for the next message, unless its last load failed: then
            // the user selects it again after fixing what `status` reports.
            let app = app.clone();
            app.schedule_models.models.clone().start(&key, move |cancel| async move { load(&app, &selected, cancel).await });
            return None;
        };
        let parts = super::classifier::parts(text);
        let vectors = loaded.encoder.encode(&parts, cancel).await.ok()?;
        let (index, score) = loaded.classifier.best(&vectors)?;
        let words: String = parts[index].chars().take(MAX_QUOTE_CHARS).collect();
        Some((words, score))
    }
    #[cfg(not(feature = "embedding-local"))]
    {
        let _ = (app, text, cancel);
        None
    }
}

#[cfg(test)]
#[path = "runner_test.rs"]
mod tests;
