//! Privacy-safe setup evidence. No account snapshot or remote error text enters this report.
use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};

use crate::{app::App, credentials::PROVIDER_KINDS, relay::RelayError};

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum RelayReason {
    None,
    NotConfigured,
    InvalidUrl,
    Unreachable,
    UpdateRequired,
    HttpError,
}

fn reason(error: &RelayError) -> RelayReason {
    match error.status {
        Some(426) => RelayReason::UpdateRequired,
        Some(_) => RelayReason::HttpError,
        None => RelayReason::Unreachable,
    }
}

fn os(value: &str) -> &'static str {
    match value {
        "macos" => "macos",
        "linux" => "linux",
        "windows" => "windows",
        "ios" => "ios",
        "ipados" => "ipados",
        "android" => "android",
        _ => "unknown",
    }
}

// Paired Device versions are untrusted text, not build constants. Only numeric releases
// are disclosed; arbitrary suffixes could contain private names or remote error text.
fn version(value: &str) -> Option<&str> {
    let mut parts = value.split('.');
    let valid = (0..3).all(|_| parts.next().is_some_and(|p| !p.is_empty() && p.len() <= 10 && p.bytes().all(|b| b.is_ascii_digit())))
        && parts.next().is_none();
    valid.then_some(value)
}

fn plugin_state(value: &str) -> &'static str {
    match value {
        "ready" => "ready",
        "needs_setup" => "needs_setup",
        "needs_auth" => "needs_auth",
        "connecting" => "connecting",
        "error" => "error",
        _ => "unknown",
    }
}

/// The relay host and explicit port are deliberately disclosed so a failing selection
/// can be identified. No userinfo, path, query, fragment or raw error is returned.
/// This probes only public relay health, never providers or plugins, and uploads nothing.
pub async fn report(app: &Arc<App>) -> Value {
    let relay = match app.relay_url() {
        None => json!({"configured": false, "host": null, "reachable": null, "reason": RelayReason::NotConfigured, "http_status": null, "protocol": null}),
        Some(url) => match reqwest::Url::parse(&url).ok().filter(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some()) {
            None => json!({"configured": true, "host": null, "reachable": null, "reason": RelayReason::InvalidUrl, "http_status": null, "protocol": null}),
            Some(parsed) => {
                let host = match parsed.port() {
                    Some(port) => format!("{}:{port}", parsed.host().unwrap()),
                    None => parsed.host().unwrap().to_string(),
                };
                // Preserve the selected relay URL and health semantics; do not invent a
                // different endpoint by removing a configured reverse-proxy path.
                match app.relay.health(&url).await {
                    Ok(protocol) => json!({"configured": true, "host": host, "reachable": true, "reason": RelayReason::None, "http_status": null, "protocol": protocol}),
                    Err(error) => json!({"configured": true, "host": host, "reachable": error.status.is_some(), "reason": reason(&error), "http_status": error.status, "protocol": null}),
                }
            }
        },
    };
    let providers = {
        let credentials = app.credentials.lock().unwrap();
        let built_in: Vec<Value> = PROVIDER_KINDS.iter().map(|kind| {
            let configured = match *kind {
                "chatgpt" => credentials.chatgpt.is_some(),
                "grok" => credentials.grok.is_some(),
                _ => credentials.api_key(kind).is_some(),
            };
            json!({"kind": kind, "configured": configured})
        }).collect();
        json!({"built_in": built_in, "custom_configured": credentials.custom.len(), "health": "not_checked"})
    };
    let (plugins, mcp_json) = {
        let store = app.plugins.lock().unwrap();
        let plugins: Vec<Value> = store.statuses().iter().enumerate().map(|(index, status)|
            json!({"slot": index + 1, "state": plugin_state(&status.state)})
        ).collect();
        let mcp_json = json!({"servers": store.mcp.servers.len(), "problems": store.mcp.servers.iter().filter(|s| s.problem().is_some()).count(), "file_error": store.mcp.error.is_some()});
        (json!({"basis": "cached_setup_state", "entries": plugins}), mcp_json)
    };
    let this_id = app.this_device_id();
    let this_os = app.machine.lock().unwrap().as_ref().map(|m| os(&m.os)).unwrap_or_else(|| os(if std::env::consts::OS == "macos" { "macos" } else { std::env::consts::OS }));
    let runners: Vec<Value> = {
        let state = app.state.lock().unwrap();
        state.devices.iter().filter(|d| d.is_runner()).map(|d| {
            let is_this = this_id.as_deref() == Some(d.id.as_str());
            json!({"is_this_device": is_this, "os": os(&d.os), "presence": if is_this || state.device_online.contains(&d.id) { "online" } else { "offline" }, "version": version(&d.version)})
        }).collect()
    };
    json!({
        "schema_version": 1,
        "versions": {"core": crate::config::VERSION, "relay_protocol": crate::relay::PROTOCOL},
        "this_device": {"os": this_os, "is_runner": matches!(this_os, "macos" | "linux" | "windows"), "has_identity": app.has_identity()},
        "home": {"exists": app.config.home.is_dir()},
        "port": {"free": std::net::TcpListener::bind(("127.0.0.1", app.config.port)).is_ok()},
        "relay": relay, "providers": providers, "plugins": plugins, "mcp_json": mcp_json, "runners": runners,
    })
}
