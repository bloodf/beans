//! Explicit inactive copies on one Runner; approvals are bounded, expiring and single-use.
use std::path::{Component, Path};
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::app::{App, State};

const AUTHORITY: &str = "skill_library_authority_changed";
const INVALID: &str = "invalid_skill_library_request";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewRequest { version: u8, runner_id: String, root: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportRequest { version: u8, runner_id: String, token: String, confirmed: bool }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UninstallRequest { version: u8, runner_id: String, managed_id: String, confirmed: bool }

enum Operation { Preview(PreviewRequest), Import(ImportRequest), Uninstall(UninstallRequest) }
impl Operation {
    fn runner(&self) -> &str {
        match self { Self::Preview(s) => &s.runner_id, Self::Import(s) => &s.runner_id, Self::Uninstall(s) => &s.runner_id }
    }
}
fn parse(method: &str, body: Value) -> Result<Operation, String> {
    let op = match method {
        "skills.library.preview" => {
            let s: PreviewRequest = serde_json::from_value(body).map_err(|_| INVALID)?;
            if s.version != 1 || s.root.len() > 4096 || s.root.chars().any(char::is_control) || !Path::new(&s.root).is_absolute()
                || Path::new(&s.root).components().any(|c| !matches!(c, Component::RootDir | Component::Normal(_))) { return Err(INVALID.into()); }
            Operation::Preview(s)
        }
        "skills.library.import" => {
            let s: ImportRequest = serde_json::from_value(body).map_err(|_| INVALID)?;
            if s.version != 1 || !opaque(&s.token) { return Err(INVALID.into()); }
            if !s.confirmed { return Err("skill_library_confirmation_required".into()); }
            Operation::Import(s)
        }
        "skills.library.uninstall" => {
            let s: UninstallRequest = serde_json::from_value(body).map_err(|_| INVALID)?;
            if s.version != 1 || !opaque(&s.managed_id) { return Err(INVALID.into()); }
            if !s.confirmed { return Err("skill_library_confirmation_required".into()); }
            Operation::Uninstall(s)
        }
        _ => return Err(INVALID.into()),
    };
    if op.runner().is_empty() || op.runner().len() > 1024 { return Err(INVALID.into()); }
    Ok(op)
}
fn opaque(value: &str) -> bool { value.len() == 32 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) }

#[derive(Clone)]
struct Authority { incarnation: u64, actor: String, runner: String, binding: (i64, i64, String) }
fn binding(state: &State, actor: &str, runner: &str) -> Result<(i64, i64, String), String> {
    let actor_time = *state.listed_machines.get(actor).ok_or(AUTHORITY)?;
    let runner_time = *state.listed_machines.get(runner).ok_or(AUTHORITY)?;
    let device = state.devices.iter().find(|d| d.id == runner && d.is_runner()).ok_or(AUTHORITY)?;
    Ok((actor_time, runner_time, device.box_pubkey.clone()))
}
impl Authority {
    fn capture(app: &App, actor: &str, runner: &str) -> Result<Self, String> {
        let lifecycle = app.plugin_admission(None).map_err(|_| AUTHORITY)?;
        let state = app.state.lock().unwrap();
        Ok(Self { incarnation: lifecycle.get().0, actor: actor.into(), runner: runner.into(), binding: binding(&state, actor, runner)? })
    }
    // Both locks remain held by the caller through publication. Never calls a state-locking helper.
    fn enter<'a>(&self, app: &'a App, local_runner: bool) -> Result<(parking_lot::ReentrantMutexGuard<'a, std::cell::Cell<(u64, bool)>>, std::sync::MutexGuard<'a, State>), String> {
        let lifecycle = app.plugin_admission(Some(self.incarnation)).map_err(|_| AUTHORITY)?;
        let state = app.state.lock().unwrap();
        if binding(&state, &self.actor, &self.runner)? != self.binding { return Err(AUTHORITY.into()); }
        let local = app.this_device_id().ok_or(AUTHORITY)?;
        if local.as_str() != if local_runner { self.runner.as_str() } else { self.actor.as_str() } { return Err(AUTHORITY.into()); }
        Ok((lifecycle, state))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Summary { name: Option<String>, description: Option<String>, license: Option<String>, findings: Vec<String>, file_count: usize, total_bytes: u64 }
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewReply { version: u8, token: String, expires_in_seconds: u64, runner_id: String, source: String, destination: String, summary: Summary }
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportReply { version: u8, runner_id: String, managed_id: String, summary: Summary, durability_warning: bool }
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UninstallReply { version: u8, runner_id: String, managed_id: String, cleanup_warning: bool, durability_warning: bool }
fn text(value: &str, max: usize) -> bool { value.len() <= max && !value.chars().any(char::is_control) }
fn valid_summary(s: &Summary) -> bool {
    s.file_count > 0 && s.file_count <= 256 && s.total_bytes <= 16 * 1024 * 1024
        && s.name.as_deref().is_none_or(|s| text(s, 256)) && s.description.as_deref().is_none_or(|s| text(s, 2048))
        && s.license.as_deref().is_none_or(|s| text(s, 256)) && s.findings.len() <= 64 && s.findings.iter().all(|s| text(s, 512))
}
fn summary_record(value: &Value) -> Result<(), String> {
    let object = value.as_object().ok_or("invalid_skill_library_response")?;
    let keys = ["name", "description", "license", "findings", "file_count", "total_bytes"];
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
        return Err("invalid_skill_library_response".into());
    }
    Ok(())
}
fn decode(method: &str, value: Value, runner: &str, requested_removal: Option<&str>) -> Result<Value, String> {
    let invalid = || "invalid_skill_library_response".to_string();
    match method {
        "skills.library.preview" => {
            summary_record(&value["summary"])?;
            let r: PreviewReply = serde_json::from_value(value).map_err(|_| invalid())?;
            if r.version != 1 || r.runner_id != runner || !opaque(&r.token) || r.expires_in_seconds == 0 || r.expires_in_seconds > 300
                || r.source.is_empty() || !text(&r.source, 4096) || r.destination.is_empty() || !text(&r.destination, 4096) || !valid_summary(&r.summary) { return Err(invalid()); }
            serde_json::to_value(r).map_err(|_| invalid())
        }
        "skills.library.import" => {
            summary_record(&value["summary"])?;
            let r: ImportReply = serde_json::from_value(value).map_err(|_| invalid())?;
            if r.version != 1 || r.runner_id != runner || !opaque(&r.managed_id) || !valid_summary(&r.summary) { return Err(invalid()); }
            serde_json::to_value(r).map_err(|_| invalid())
        }
        "skills.library.uninstall" => {
            let r: UninstallReply = serde_json::from_value(value).map_err(|_| invalid())?;
            if r.version != 1 || r.runner_id != runner || !opaque(&r.managed_id) || requested_removal != Some(r.managed_id.as_str()) { return Err(invalid()); }
            serde_json::to_value(r).map_err(|_| invalid())
        }
        _ => Err(invalid()),
    }
}

pub async fn dispatch(app: &Arc<App>, method: &str, body: Value) -> Result<Value, String> {
    let operation = parse(method, body.clone())?;
    let actor = app.this_device_id().ok_or("identity_required")?;
    let authority = Authority::capture(app, &actor, operation.runner())?;
    let result = if actor == operation.runner() { serve_as(app, method, body, &actor).await? }
        else { crate::requests::ask(app, operation.runner(), method, body).await? };
    let _guards = authority.enter(app, false)?;
    let requested_removal = match &operation { Operation::Uninstall(s) => Some(s.managed_id.as_str()), _ => None };
    let result = decode(method, result, operation.runner(), requested_removal)?;
    if let Operation::Preview(s) = operation { if result["source"] != s.root { return Err("invalid_skill_library_response".into()); } }
    Ok(result)
}

#[cfg(test)]
mod response_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_missing_nullable_summary_and_wrong_removal_id() {
        let runner = "synthetic-runner";
        let id = "a".repeat(32);
        let summary = json!({"name":null,"description":null,"license":null,"findings":[],"file_count":1,"total_bytes":3});
        let preview = json!({"version":1,"token":id,"expires_in_seconds":300,"runner_id":runner,"source":"/selected","destination":"/owned/skill-library","summary":summary});
        let imported = json!({"version":1,"runner_id":runner,"managed_id":id,"summary":summary,"durability_warning":false});
        for (method, valid) in [("skills.library.preview", preview), ("skills.library.import", imported)] {
            assert_eq!(decode(method, valid.clone(), runner, None).unwrap(), valid);
            for key in ["name", "description", "license", "findings", "file_count", "total_bytes"] {
                let mut missing = valid.clone();
                missing["summary"].as_object_mut().unwrap().remove(key);
                assert_eq!(decode(method, missing, runner, None).unwrap_err(), "invalid_skill_library_response", "missing {key} in {method}");
            }
        }
        let removed = json!({"version":1,"runner_id":runner,"managed_id":id,"cleanup_warning":false,"durability_warning":false});
        assert_eq!(decode("skills.library.uninstall", removed.clone(), runner, Some(&id)).unwrap(), removed);
        let mut wrong = removed;
        wrong["managed_id"] = json!("b".repeat(32));
        assert_eq!(decode("skills.library.uninstall", wrong, runner, Some(&id)).unwrap_err(), "invalid_skill_library_response");
    }
}

pub async fn serve_as(app: &Arc<App>, method: &str, body: Value, actor: &str) -> Result<Value, String> {
    let operation = parse(method, body)?;
    let authority = Authority::capture(app, actor, operation.runner())?;
    { let _guards = authority.enter(app, true)?; }
    #[cfg(feature = "runner")]
    {
        let app = app.clone();
        tokio::task::spawn_blocking(move || runner::perform(&app, operation, authority)).await.map_err(|_| "skill_library_failed".to_string())?
    }
    #[cfg(not(feature = "runner"))]
    { let _ = (operation, authority); Err("secure_skill_library_unavailable".into()) }
}

#[cfg(feature = "runner")]
mod runner {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock, Weak};
    use std::time::{Duration, Instant};
    use beans_agent::harness::skill_library::{SkillLibrary, SourcePreview};

    struct Approval { app: Weak<App>, authority: Authority, source: String, inventory: SourcePreview, expires: Instant }
    // No App field/helper change: weak ownership prevents keeping forgotten accounts alive.
    static APPROVALS: OnceLock<Mutex<HashMap<String, Approval>>> = OnceLock::new();
    fn approvals() -> &'static Mutex<HashMap<String, Approval>> { APPROVALS.get_or_init(|| Mutex::new(HashMap::new())) }
    fn summary(p: &SourcePreview) -> Summary {
        let field = |key: &str, max| p.metadata.get(key).filter(|s| text(s, max)).cloned();
        Summary { name: field("name", 256), description: field("description", 2048), license: p.license.as_ref().filter(|s| text(s, 256)).cloned(),
            findings: p.findings.iter().filter(|s| text(s, 512)).take(64).cloned().collect(), file_count: p.files.len(), total_bytes: p.files.iter().map(|f| f.bytes).sum() }
    }
    fn failure(e: std::io::Error) -> String {
        if e.kind() == std::io::ErrorKind::Unsupported { "secure_skill_library_unavailable".into() }
        else if e.to_string() == AUTHORITY { AUTHORITY.into() }
        else if e.to_string() == "selected source changed since approval" { "skill_library_source_changed".into() }
        else { "skill_library_failed".into() }
    }
    fn library(app: &App) -> Result<SkillLibrary, String> {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::DirBuilderExt;
            let root = app.config.home.join("skill-library");
            match std::fs::DirBuilder::new().mode(0o700).create(&root) {
                Ok(()) => {}, Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}, Err(e) => return Err(failure(e)),
            }
            SkillLibrary::open(&root).map_err(failure)
        }
        #[cfg(not(target_os = "linux"))]
        { let _ = app; Err("secure_skill_library_unavailable".into()) }
    }
    pub(super) fn perform(app: &Arc<App>, operation: Operation, authority: Authority) -> Result<Value, String> {
        match operation {
            Operation::Preview(s) => {
                let library = { let _guards = authority.enter(app, true)?; library(app)? };
                let inventory = library.preview_source(Path::new(&s.root)).map_err(failure)?;
                let _guards = authority.enter(app, true)?;
                let mut pending = approvals().lock().unwrap();
                pending.retain(|_, p| p.expires > Instant::now() && p.app.strong_count() > 0);
                if pending.len() >= 32 { return Err("skill_library_too_many_approvals".into()); }
                let token = uuid::Uuid::new_v4().simple().to_string();
                let reply = PreviewReply { version: 1, token: token.clone(), expires_in_seconds: 300, runner_id: s.runner_id,
                    source: s.root.clone(), destination: app.config.home.join("skill-library").to_str().ok_or(INVALID)?.into(), summary: summary(&inventory) };
                pending.insert(token, Approval { app: Arc::downgrade(app), authority: authority.clone(), source: s.root, inventory, expires: Instant::now() + Duration::from_secs(300) });
                serde_json::to_value(reply).map_err(|_| "skill_library_failed".into())
            }
            Operation::Import(s) => {
                // Consume before every effect, including refusal due to stale authority or source.
                let approval = approvals().lock().unwrap().remove(&s.token).ok_or("skill_library_approval_not_found")?;
                if approval.expires <= Instant::now() || !approval.app.ptr_eq(&Arc::downgrade(app))
                    || approval.authority.actor != authority.actor || approval.authority.runner != authority.runner
                    || approval.authority.incarnation != authority.incarnation || approval.authority.binding != authority.binding { return Err("skill_library_approval_stale".into()); }
                { let _guards = approval.authority.enter(app, true)?; }
                let library = { let _guards = approval.authority.enter(app, true)?; library(app)? };
                let outcome = library.import_approved(Path::new(&approval.source), &authority.runner, &approval.inventory,
                    || approval.authority.enter(app, true).map_err(std::io::Error::other)).map_err(failure)?;
                let _guards = authority.enter(app, true)?;
                serde_json::to_value(ImportReply { version: 1, runner_id: s.runner_id, managed_id: outcome.manifest.bundle_id,
                    summary: summary(&approval.inventory), durability_warning: outcome.durability_warning.is_some() }).map_err(|_| "skill_library_failed".into())
            }
            Operation::Uninstall(s) => {
                let library = { let _guards = authority.enter(app, true)?; library(app)? };
                let outcome = library.uninstall_guarded(&s.managed_id, || authority.enter(app, true).map_err(std::io::Error::other)).map_err(failure)?;
                let _guards = authority.enter(app, true)?;
                serde_json::to_value(UninstallReply { version: 1, runner_id: s.runner_id, managed_id: s.managed_id,
                    cleanup_warning: outcome.cleanup_warning.is_some(), durability_warning: outcome.durability_warning.is_some() }).map_err(|_| "skill_library_failed".into())
            }
        }
    }
}
