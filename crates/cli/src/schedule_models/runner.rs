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
use crate::embeddings::assets::{AssetInstaller, AssetKind, AssetPlan, AssetRequest, AssetSource, InstalledAssets, RedirectHosts};

const STATE_FILE: &str = "schedule-models/state.json";
const ASSET_DIR: &str = "schedule-models/assets";
const APPROVAL_SECS: u64 = 600;
const MAX_APPROVALS: usize = 16;
/// The largest ONNX Runtime library a user may supply, as local memory embeddings allow.
const MAX_RUNTIME_BYTES: u64 = 256 * 1024 * 1024;
const RUNTIME_LICENSE: &str = "ONNX Runtime (MIT), supplied by the user";
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

/// What the Runner keeps between runs, for one account.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    account: String,
    mode: Detection,
    selected: Option<String>,
    installs: BTreeMap<String, Install>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Install {
    assets: InstalledAssets,
    /// Trained for these files on `training.json`, once.
    classifier: Option<Classifier>,
    installed_at: f64,
}

#[cfg_attr(not(feature = "embedding-local"), allow(dead_code))]
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

/// Where catalog files download from. Tests point it at a local server.
#[derive(Clone)]
pub(crate) struct DownloadPolicy {
    base: String,
    allow_insecure_http: bool,
    redirects: RedirectHosts,
}

impl Default for DownloadPolicy {
    fn default() -> Self {
        let hosts = |names: &[&str]| names.iter().map(|n| n.to_string()).collect();
        DownloadPolicy {
            base: super::HUGGING_FACE.into(),
            allow_insecure_http: false,
            // Hugging Face sends file downloads on to its CDN (cdn-lfs.hf.co, cas-bridge.xethub.hf.co).
            redirects: RedirectHosts {
                exact: hosts(&["huggingface.co", "hf.co"]),
                parents: hosts(&["huggingface.co", "hf.co"]),
            },
        }
    }
}

/// A model ready to read messages.
#[cfg(feature = "embedding-local")]
struct Loaded {
    account: String,
    model_id: String,
    fingerprint: String,
    encoder: super::engine::Encoder,
    classifier: Classifier,
}

/// The Runner's schedule-model state in memory.
#[derive(Default)]
pub struct Runtime {
    approvals: Mutex<HashMap<String, Approval>>,
    installer: Mutex<Option<(String, Arc<AssetInstaller>)>>,
    saved: Mutex<Option<Saved>>,
    /// The catalog in place of the built-in one, for tests.
    catalog: Mutex<Option<&'static Catalog>>,
    /// Held through each read, change, and write of the saved state.
    edit: Mutex<()>,
    policy: Mutex<Option<DownloadPolicy>>,
    /// Cancels installs and loads when the account pauses or is forgotten.
    cancel: Mutex<CancellationToken>,
    #[cfg(feature = "embedding-local")]
    loaded: Mutex<Option<Arc<Loaded>>>,
    /// The model a load is under way for.
    loading: Mutex<Option<String>>,
    /// Why the selected model did not load, until it does.
    load_error: Mutex<Option<String>>,
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
        *self.saved.lock() = None;
        #[cfg(feature = "embedding-local")]
        {
            *self.loaded.lock() = None;
        }
        *self.loading.lock() = None;
        *self.load_error.lock() = None;
    }

    fn cancel_token(&self) -> CancellationToken {
        self.cancel.lock().child_token()
    }

    fn policy(&self) -> DownloadPolicy {
        self.policy.lock().clone().unwrap_or_default()
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
    pub(crate) fn set_policy(&self, base: &str, allow_insecure_http: bool, redirects: RedirectHosts) {
        *self.policy.lock() = Some(DownloadPolicy {
            base: base.into(),
            allow_insecure_http,
            redirects,
        });
        *self.installer.lock() = None;
    }
}

/// The catalog this Runner offers.
fn catalog(app: &App) -> &'static Catalog {
    app.schedule_models.catalog.lock().unwrap_or_else(super::catalog)
}

fn account(app: &App) -> Result<String, String> {
    app.machine_file().map(|m| m.identity_pubkey).ok_or_else(|| "identity_required".into())
}

fn state_path(app: &App) -> PathBuf {
    app.config.home.join(STATE_FILE)
}

/// The saved state for the current account; another account's reads as nothing saved.
fn saved(app: &App) -> Result<Saved, String> {
    let account = account(app)?;
    let mut cache = app.schedule_models.saved.lock();
    if let Some(saved) = cache.as_ref().filter(|s| s.account == account) {
        return Ok(saved.clone());
    }
    let saved = match std::fs::read(state_path(app)) {
        Ok(bytes) => serde_json::from_slice::<Saved>(&bytes).ok().filter(|s| s.account == account),
        Err(_) => None,
    }
    .unwrap_or(Saved { account, ..Saved::default() });
    *cache = Some(saved.clone());
    Ok(saved)
}

/// Reads, changes, and writes the saved state as one step, while `still` holds.
fn edit<T>(app: &App, still: impl Fn() -> Result<(), String>, change: impl FnOnce(&mut Saved) -> Result<T, String>) -> Result<T, String> {
    let _edit = app.schedule_models.edit.lock();
    still()?;
    let mut saved = saved(app)?;
    let result = change(&mut saved)?;
    crate::config::write_json_private(&state_path(app), &saved).map_err(|_| "storage_failed".to_string())?;
    *app.schedule_models.saved.lock() = Some(saved);
    Ok(result)
}

/// Who asked, for which account, and whether both still hold: the account is the same, the
/// requester is this Device or still paired, and the account is not paused.
struct Authority {
    actor: String,
    account: String,
    incarnation: u64,
}

impl Authority {
    fn capture(app: &App, actor: &str) -> Result<Authority, String> {
        let incarnation = app.plugin_admission(None).map_err(|_| "authority_changed".to_string())?.get().0;
        let authority = Authority {
            actor: actor.into(),
            account: account(app)?,
            incarnation,
        };
        authority.check(app)?;
        Ok(authority)
    }

    fn check(&self, app: &App) -> Result<(), String> {
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
        _ => Err("unknown_schedule_models_method".into()),
    }
}

fn list(app: &App) -> Result<Value, String> {
    let saved = saved(app)?;
    let catalog = catalog(app);
    let models: Vec<Value> = catalog
        .models
        .iter()
        .map(|model| {
            let mut listing = model.listing();
            listing["installed"] = saved.installs.contains_key(&model.id).into();
            listing["selected"] = (saved.selected.as_deref() == Some(model.id.as_str())).into();
            listing
        })
        .collect();
    Ok(json!({
        "note": catalog.note,
        "max_bytes": MAX_MODEL_BYTES,
        "models": models,
        "mode": saved.mode,
        "selected": saved.selected,
        "runs_models": cfg!(feature = "embedding-local"),
        "onnx_runtime": "ONNX models need the ONNX Runtime shared library (onnxruntime.dll, libonnxruntime.dylib, or libonnxruntime.so), version 1.27 or newer, CPU build; pass its path as `runtime` to preview. Static models need nothing else.",
    }))
}

fn status(app: &App) -> Result<Value, String> {
    let saved = saved(app)?;
    Ok(json!({
        "mode": saved.mode,
        "selected": saved.selected,
        "installed": saved.installs.keys().collect::<Vec<_>>(),
        "state": load_state(app, &saved),
        "error": app.schedule_models.load_error.lock().clone(),
    }))
}

/// Whether the selected model is ready to read messages.
fn load_state(app: &App, saved: &Saved) -> &'static str {
    let Some(selected) = &saved.selected else { return "none_selected" };
    if !cfg!(feature = "embedding-local") {
        return "runtime_unavailable";
    }
    #[cfg(feature = "embedding-local")]
    if app.schedule_models.loaded.lock().as_ref().is_some_and(|l| &l.model_id == selected && l.account == saved.account) {
        return "ready";
    }
    if app.schedule_models.loading.lock().as_deref() == Some(selected.as_str()) {
        return "loading";
    }
    if app.schedule_models.load_error.lock().is_some() {
        return "failed";
    }
    let _ = app;
    "not_loaded"
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
    let policy = app.schedule_models.policy();
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
                        url: format!("{}/{}/resolve/{}/{}", policy.base, model.repo, model.revision, file.path),
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
    match (model.engine, &request.runtime) {
        (Engine::Onnx, Some(path)) => {
            let (bytes, sha256) = hash_file(path, MAX_RUNTIME_BYTES, &cancel).await?;
            assets.push(AssetRequest {
                kind: AssetKind::Runtime,
                source: AssetSource::Supplied { path: path.clone() },
                license: RUNTIME_LICENSE.into(),
                bytes,
                sha256,
            });
        }
        (Engine::Onnx, None) => return Err("runtime_required".into()),
        (Engine::Static, Some(_)) => return Err("runtime_not_needed".into()),
        (Engine::Static, None) => {}
    }
    let kinds: Vec<AssetKind> = assets.iter().map(|a| a.kind).collect();
    let installer = installer(app, &policy)?;
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
        "total_bytes": preview.total_bytes,
        "assets": preview.assets,
    }))
}

fn kind_of(role: &str) -> AssetKind {
    match role {
        "weights" => AssetKind::Weights,
        "model" => AssetKind::Model,
        _ => AssetKind::Tokenizer,
    }
}

/// One installer per account, downloading only from the policy's base origin.
fn installer(app: &App, policy: &DownloadPolicy) -> Result<Arc<AssetInstaller>, String> {
    let account = account(app)?;
    let mut installer = app.schedule_models.installer.lock();
    if let Some((_, installer)) = installer.as_ref().filter(|(owner, _)| *owner == account) {
        return Ok(installer.clone());
    }
    let origin = reqwest::Url::parse(&policy.base).map_err(|_| "invalid_asset_policy")?.origin().ascii_serialization();
    let namespace = format!("{:x}", Sha256::digest(format!("beans-schedule-models\0{account}").as_bytes()));
    let root = app.config.home.join(ASSET_DIR).join(&namespace[..32]);
    let created = Arc::new(AssetInstaller::with_redirects(root, [origin].into(), policy.allow_insecure_http, policy.redirects.clone()).map_err(|_| "invalid_asset_policy")?);
    *installer = Some((account, created.clone()));
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
    let installed = approval
        .installer
        .apply_assets(&approval.token, &approval.digest, true, cancel.clone())
        .await
        .map_err(|_| "asset_install_failed")?;
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
    let state = prepare_now(app, &approval.model_id, cancel).await;
    Ok(json!({ "installed": true, "model_id": approval.model_id, "state": state }))
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
            saved.mode = request.mode;
            saved.selected = request.model_id.clone();
            Ok(saved.clone())
        },
    )?;
    *app.schedule_models.load_error.lock() = None;
    let state = match &request.model_id {
        Some(id) => prepare_now(app, id, app.schedule_models.cancel_token()).await,
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
    {
        let mut loaded = app.schedule_models.loaded.lock();
        if loaded.as_ref().is_some_and(|l| l.model_id == request.model_id) {
            *loaded = None;
        }
    }
    remove_files(&install.assets);
    Ok(json!({ "removed": true, "mode": saved.mode, "selected": saved.selected }))
}

/// Loads `model_id`, training its classifier the first time, and says how that went.
async fn prepare_now(app: &Arc<App>, model_id: &str, cancel: CancellationToken) -> String {
    #[cfg(feature = "embedding-local")]
    {
        *app.schedule_models.loading.lock() = Some(model_id.to_string());
        let result = load(app, model_id, cancel).await;
        *app.schedule_models.loading.lock() = None;
        match result {
            Ok(()) => {
                *app.schedule_models.load_error.lock() = None;
                "ready".into()
            }
            Err(error) => {
                tracing::warn!(%error, model = model_id, "loading a schedule model");
                *app.schedule_models.load_error.lock() = Some(error.clone());
                error
            }
        }
    }
    #[cfg(not(feature = "embedding-local"))]
    {
        let _ = (app, model_id, cancel);
        "runtime_unavailable".into()
    }
}

#[cfg(feature = "embedding-local")]
async fn load(app: &Arc<App>, model_id: &str, cancel: CancellationToken) -> Result<(), String> {
    use super::classifier::{parts, training_messages};
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
            let account = saved.account.clone();
            edit(
                app,
                || {
                    if crate::schedule_models::runner::account(app)? == account {
                        Ok(())
                    } else {
                        Err("authority_changed".into())
                    }
                },
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
    if cancel.is_cancelled() {
        return Err("cancelled".into());
    }
    *app.schedule_models.loaded.lock() = Some(Arc::new(Loaded {
        account: saved.account,
        model_id: model_id.to_string(),
        fingerprint: install.fingerprint(),
        encoder,
        classifier,
    }));
    Ok(())
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
        let install = saved.installs.get(&selected)?;
        let loaded = app
            .schedule_models
            .loaded
            .lock()
            .clone()
            .filter(|l| l.model_id == selected && l.account == saved.account && l.fingerprint == install.fingerprint());
        let Some(loaded) = loaded else {
            start_loading(app, &selected);
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

/// Loads the selected model in the background, once at a time, unless its last load failed:
/// then the user selects it again after fixing what `status` reports.
#[cfg(feature = "embedding-local")]
fn start_loading(app: &Arc<App>, model_id: &str) {
    if app.schedule_models.load_error.lock().is_some() {
        return;
    }
    {
        let mut loading = app.schedule_models.loading.lock();
        if loading.is_some() {
            return;
        }
        *loading = Some(model_id.to_string());
    }
    let app = app.clone();
    let model_id = model_id.to_string();
    tokio::spawn(async move {
        let cancel = app.schedule_models.cancel_token();
        let _ = prepare_now(&app, &model_id, cancel).await;
    });
}

#[cfg(test)]
#[path = "runner_test.rs"]
mod tests;
