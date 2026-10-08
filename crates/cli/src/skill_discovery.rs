//! Explicit, read-only Runner discovery. Scanner diagnostics never cross the API verbatim.
use std::path::{Component, Path};
use std::sync::Arc;
use serde::Deserialize;
use serde_json::{json, Value};
use crate::app::App;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection { version: u8, runner_id: String, root: String }

fn selection(body: Value) -> Result<Selection, String> {
    let s: Selection = serde_json::from_value(body).map_err(|_| "invalid_skill_discovery_request")?;
    if s.version != 1 || s.runner_id.is_empty() || s.runner_id.len() > 1024 || s.root.len() > 4096
        || s.root.contains('\0') || !Path::new(&s.root).is_absolute()
        || Path::new(&s.root).components().any(|c| !matches!(c, Component::RootDir | Component::Prefix(_) | Component::Normal(_))) {
        return Err("invalid_skill_discovery_request".into());
    }
    Ok(s)
}

fn binding(app: &App, actor: &str, runner: &str) -> Result<(i64, i64, String), String> {
    let state = app.state.lock().unwrap();
    if !state.listed_machines.contains_key(actor) || !state.listed_machines.contains_key(runner)
        || !state.devices.iter().any(|d| d.id == runner && d.is_runner()) {
        return Err("skill_discovery_authority_changed".into());
    }
    let device = state.devices.iter().find(|d| d.id == runner).ok_or("skill_discovery_authority_changed")?;
    Ok((state.listed_machines[actor], state.listed_machines[runner], device.box_pubkey.clone()))
}

pub async fn dispatch(app: &Arc<App>, body: Value) -> Result<Value, String> {
    let s = selection(body.clone())?;
    let actor = app.this_device_id().ok_or("identity_required")?;
    let incarnation = { app.plugin_admission(None)?.get().0 };
    let admitted = binding(app, &actor, &s.runner_id)?;
    let result = if actor == s.runner_id {
        serve_as(app, body, &actor).await?
    } else {
        crate::requests::ask(app, &s.runner_id, "skills.discovery", body).await.map_err(|_| "skill_discovery_request_failed")?
    };
    let _guard = app.plugin_admission(Some(incarnation)).map_err(|_| "skill_discovery_authority_changed")?;
    if app.this_device_id().as_deref() != Some(&actor) { return Err("skill_discovery_authority_changed".into()); }
    if binding(app, &actor, &s.runner_id)? != admitted { return Err("skill_discovery_authority_changed".into()); }
    Ok(result)
}

pub async fn serve_as(app: &Arc<App>, body: Value, actor: &str) -> Result<Value, String> {
    let s = selection(body)?;
    let incarnation = { app.plugin_admission(None)?.get().0 };
    let admitted = binding(app, actor, &s.runner_id)?;
    if app.this_device_id().as_deref() != Some(&s.runner_id) { return Err("skill_discovery_wrong_runner".into()); }
    #[cfg(feature = "runner")]
    let result = {
        let root = s.root;
        tokio::task::spawn_blocking(move || project(beans_agent::harness::skill_discovery::discover_skills(Path::new(&root))))
            .await.map_err(|_| "skill_discovery_failed")?
    };
    #[cfg(not(feature = "runner"))]
    let result = json!({"version":1,"status":"unsupported","skills":[],"diagnostics":[{"path":null,"code":"secure_discovery_unavailable"}]});
    let _guard = app.plugin_admission(Some(incarnation)).map_err(|_| "skill_discovery_authority_changed")?;
    if binding(app, actor, &s.runner_id)? != admitted { return Err("skill_discovery_authority_changed".into()); }
    if app.this_device_id().as_deref() != Some(&s.runner_id) { return Err("skill_discovery_authority_changed".into()); }
    Ok(result)
}

#[cfg(feature = "runner")]
fn label(path: &Path) -> Option<&str> {
    let text = path.to_str()?;
    (text.len() <= 4096 && !text.chars().any(char::is_control)
        && path.components().all(|c| matches!(c, Component::Normal(_)))).then_some(text)
}

#[cfg(feature = "runner")]
fn project(scan: beans_agent::harness::skill_discovery::SkillDiscovery) -> Value {
    let unavailable = !cfg!(target_os = "linux") || scan.diagnostics.iter().any(|d| d.message.starts_with("secure root open failed"));
    let skills: Vec<_> = scan.skills.iter().filter_map(|s| {
        let path = label(&s.path)?;
        let field = |key: &str, max| s.fields.get(key).filter(|v| v.len() <= max && !v.chars().any(char::is_control));
        Some(json!({"path":path,"name":field("name",256),"description":field("description",2048),"license":field("license",256)}))
    }).collect();
    let diagnostics: Vec<_> = scan.diagnostics.iter().take(4096).map(|d| {
        let code = if unavailable { "secure_discovery_unavailable" }
            else if d.message.starts_with("unsupported field") || d.message.starts_with("structured YAML") { "unsupported_metadata" }
            else if d.message.contains("limit") { "discovery_limit" }
            else if d.message.starts_with("missing ") { "missing_metadata" }
            else { "path_not_inspected" };
        json!({"path":label(&d.path),"code":code})
    }).collect();
    json!({"version":1,"status":if unavailable {"unsupported"} else {"scanned"},"skills":skills,"diagnostics":diagnostics})
}
