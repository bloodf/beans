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
    decode_response(result)
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

fn record<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a serde_json::Map<String, Value>, String> {
    let object = value.as_object().ok_or("invalid_skill_discovery_response")?;
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
        return Err("invalid_skill_discovery_response".into());
    }
    Ok(object)
}

fn scalar(value: &Value, max: usize, nullable: bool) -> Result<Value, String> {
    if nullable && value.is_null() { return Ok(Value::Null); }
    let text = value.as_str().ok_or("invalid_skill_discovery_response")?;
    if text.len() > max || text.chars().any(char::is_control) { return Err("invalid_skill_discovery_response".into()); }
    Ok(Value::String(text.into()))
}

fn relative(value: &Value, nullable: bool) -> Result<Value, String> {
    let result = scalar(value, 4096, nullable)?;
    if let Some(text) = result.as_str() {
        if text.is_empty() || text.contains(['\\', ':']) || text.split('/').any(|part| matches!(part, "" | "." | "..")) {
            return Err("invalid_skill_discovery_response".into());
        }
    }
    Ok(result)
}

fn decode_response(value: Value) -> Result<Value, String> {
    let object = record(&value, &["version", "status", "skills", "diagnostics"])?;
    if object["version"].as_u64() != Some(1) || !matches!(object["status"].as_str(), Some("scanned" | "unsupported")) {
        return Err("invalid_skill_discovery_response".into());
    }
    let skills = object["skills"].as_array().ok_or("invalid_skill_discovery_response")?;
    let diagnostics = object["diagnostics"].as_array().ok_or("invalid_skill_discovery_response")?;
    if skills.len() > 256 || diagnostics.len() > 4096 || (object["status"] == "unsupported" && !skills.is_empty()) {
        return Err("invalid_skill_discovery_response".into());
    }
    let skills = skills.iter().map(|row| {
        let row = record(row, &["path", "name", "description", "license"])?;
        Ok(json!({"path":relative(&row["path"],false)?,"name":scalar(&row["name"],256,true)?,"description":scalar(&row["description"],2048,true)?,"license":scalar(&row["license"],256,true)?}))
    }).collect::<Result<Vec<Value>, String>>()?;
    let diagnostics = diagnostics.iter().map(|row| {
        let row = record(row, &["path", "code"])?;
        if !matches!(row["code"].as_str(), Some("secure_discovery_unavailable" | "unsupported_metadata" | "discovery_limit" | "missing_metadata" | "path_not_inspected")) {
            return Err("invalid_skill_discovery_response".into());
        }
        Ok(json!({"path":relative(&row["path"],true)?,"code":row["code"]}))
    }).collect::<Result<Vec<Value>, String>>()?;
    Ok(json!({"version":1,"status":object["status"],"skills":skills,"diagnostics":diagnostics}))
}

#[cfg(feature = "runner")]
fn label(path: &Path) -> Option<&str> {
    let text = path.to_str()?;
    (!text.is_empty() && relative(&Value::String(text.into()), false).is_ok()).then_some(text)
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

#[cfg(test)]
mod response_tests {
    use super::*;
    #[test]
    fn rejects_malformed_metadata_replies() {
        let row = json!({"path":"nested/SKILL.md","name":null,"description":"Selected metadata","license":"MIT"});
        let valid = json!({"version":1,"status":"scanned","skills":[row],"diagnostics":[{"path":null,"code":"unsupported_metadata"}]});
        assert_eq!(decode_response(valid.clone()).unwrap(), valid);
        for path in ["/private", "../private", "a//b", "C:private", "a\\b", "a\u{85}b", ""] {
            let mut bad = valid.clone(); bad["skills"][0]["path"] = json!(path);
            assert_eq!(decode_response(bad).unwrap_err(), "invalid_skill_discovery_response");
        }
        for bad in [Value::Null, json!([]), json!({"version":1,"status":"scanned","skills":[],"diagnostics":[],"body":"PRIVATE"})] {
            assert!(decode_response(bad).is_err());
        }
        let mut bad = valid.clone(); bad["skills"] = json!(vec![row.clone();257]); assert!(decode_response(bad).is_err());
        let mut bad = valid.clone(); bad["diagnostics"] = json!(vec![json!({"path":null,"code":"path_not_inspected"});4097]); assert!(decode_response(bad).is_err());
        for (key, value) in [("name",json!({})),("name",json!("名".repeat(86))),("description",json!("x".repeat(2049))),("license",json!("\n")),("path",Value::Null),("hook",json!("PRIVATE"))] {
            let mut bad = valid.clone(); bad["skills"][0][key] = value; assert!(decode_response(bad).is_err());
        }
        let mut bad = valid.clone(); bad["diagnostics"][0]["code"] = json!("raw error"); assert!(decode_response(bad).is_err());
        let mut bad = valid.clone(); bad["skills"][0] = Value::Null; assert!(decode_response(bad).is_err());
        let mut bad = valid.clone(); bad["status"] = json!("unsupported"); assert!(decode_response(bad).is_err());
        let mut bad = valid; bad["version"] = json!(2); assert!(decode_response(bad).is_err());
    }
}
