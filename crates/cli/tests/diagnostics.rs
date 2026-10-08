use beans::{api, app::App, config::Config, diagnostics, model::Device};
use serde_json::{json, Value};

fn fixture() -> (tempfile::TempDir, std::sync::Arc<App>) {
    let home = tempfile::tempdir().unwrap();
    let app = App::load(Config { home: home.path().to_owned(), port: 0 }).unwrap();
    (home, app)
}

fn assert_private(value: &Value, home: &std::path::Path) {
    let text = value.to_string();
    assert!(!text.contains("PRIVATE_MARKER"), "{text}");
    assert!(!text.contains(home.to_str().unwrap()), "{text}");
    let keys: Vec<_> = value.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["home", "mcp_json", "plugins", "port", "providers", "relay", "runners", "schema_version", "this_device", "versions"]);
    for (field, allowed) in [
        ("home", vec!["exists"]), ("port", vec!["free"]),
        ("versions", vec!["core", "relay_protocol"]),
        ("this_device", vec!["has_identity", "is_runner", "os"]),
        ("relay", vec!["configured", "host", "http_status", "protocol", "reachable", "reason"]),
        ("providers", vec!["built_in", "custom_configured", "health"]),
        ("plugins", vec!["basis", "entries"]), ("mcp_json", vec!["file_error", "problems", "servers"]),
    ] {
        assert_eq!(value[field].as_object().unwrap().keys().map(String::as_str).collect::<Vec<_>>(), allowed);
    }
    for (rows, allowed) in [
        (&value["providers"]["built_in"], vec!["configured", "kind"]),
        (&value["plugins"]["entries"], vec!["slot", "state"]),
        (&value["runners"], vec!["is_this_device", "os", "presence", "version"]),
    ] {
        for row in rows.as_array().unwrap() {
            assert_eq!(row.as_object().unwrap().keys().map(String::as_str).collect::<Vec<_>>(), allowed);
        }
    }
}

#[tokio::test]
async fn report_excludes_private_metadata_and_distinguishes_presence() {
    let (home, app) = fixture();
    app.settings.lock().unwrap().relay_url = None;
    app.credentials.lock().unwrap().custom.insert("custom:PRIVATE_MARKER".into(), serde_json::from_value(json!({
        "name":"PRIVATE_MARKER", "api":"responses", "base_url":"http://PRIVATE_MARKER", "api_key":"PRIVATE_MARKER",
        "models":[], "created_at":0, "integration":"durindoor"
    })).unwrap());
    app.state.lock().unwrap().devices = vec![
        Device { id: "PRIVATE_MARKER".into(), name: "PRIVATE_MARKER".into(), os: "linux".into(), version: "PRIVATE_MARKER".into(), ..Default::default() },
        Device { os: "ios".into(), ..Default::default() },
    ];
    app.state.lock().unwrap().chats.push(serde_json::from_value(json!({
        "id":"PRIVATE_MARKER", "kind":"group", "title":"PRIVATE_MARKER", "bot_ids":[], "created_at":0
    })).unwrap());
    app.upsert_message(beans::model::Message::new("PRIVATE_MARKER", beans::model::Author::You, beans::model::Body::text("PRIVATE_MARKER")), false);
    std::fs::write(home.path().join("mcp.json"), r#"{"mcpServers":{"PRIVATE_MARKER":{"command":"PRIVATE_MARKER","args":[],"env":{"PRIVATE_MARKER":"PRIVATE_MARKER"}}}}"#).unwrap();
    let mut store = beans::plugins::Store::load(&app.config).unwrap();
    for status in store.statuses() {
        store.notes.insert(status.id, ("error".into(), "PRIVATE_MARKER".into()));
    }
    *app.plugins.lock().unwrap() = store;
    let report = diagnostics::report(&app).await;
    assert_private(&report, home.path());
    assert_eq!(report["providers"]["custom_configured"], 1);
    assert_eq!(report["providers"]["health"], "not_checked");
    assert_eq!(report["runners"].as_array().unwrap().len(), 1);
    assert_eq!(report["runners"][0]["presence"], "offline");
    assert!(report["runners"][0]["version"].is_null());
    assert_eq!(report["plugins"]["entries"][0]["state"], "error");
}

#[tokio::test]
async fn health_failures_never_echo_remote_errors_or_claim_unknown_machine() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for (status, body, reason) in [
        (404, "{\"error\":\"PRIVATE_MARKER\"}", "http_error"),
        (503, "PRIVATE_MARKER", "http_error"),
        (426, "PRIVATE_MARKER", "update_required"),
        (200, "PRIVATE_MARKER", "update_required"),
        (200, "{\"ok\":true,\"service\":\"PRIVATE_MARKER\",\"format\":\"beans-v2\",\"protocol\":5,\"min_protocol\":5,\"min_roster_protocol\":5,\"memory_config_version\":1}", "update_required"),
        (200, "{\"ok\":true,\"service\":\"beans-relay\",\"format\":\"beans-v2\",\"protocol\":5,\"min_protocol\":6,\"min_roster_protocol\":5,\"memory_config_version\":1}", "update_required"),
        (200, "{\"ok\":true,\"service\":\"beans-relay\",\"format\":\"beans-v2\",\"protocol\":5,\"min_protocol\":5,\"min_roster_protocol\":5,\"memory_config_version\":1}", "none"),
    ] {
        let (home, app) = fixture();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        app.settings.lock().unwrap().relay_url = Some(format!("http://{address}"));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).await.unwrap();
            socket.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let report = api::dispatch(&app, "diagnostics.report", Value::Null).await.unwrap();
        server.await.unwrap();
        assert_private(&report, home.path());
        assert_eq!(report["relay"]["host"], address.to_string());
        assert_eq!(report["relay"]["reason"], reason);
        assert_eq!(report["relay"]["reachable"], true);
    }
}

#[cfg(feature = "cli")]
#[test]
fn doctor_exports_json_and_preserves_plain_invocation() {
    let (home, app) = fixture();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let url = format!("http://PRIVATE_MARKER:PRIVATE_MARKER@127.0.0.1:{port}/PRIVATE_MARKER?token=PRIVATE_MARKER#PRIVATE_MARKER");
    app.credentials.lock().unwrap().custom.insert("custom:PRIVATE_MARKER".into(), serde_json::from_value(json!({
        "name":"PRIVATE_MARKER", "api":"responses", "base_url":"http://PRIVATE_MARKER", "api_key":"PRIVATE_MARKER",
        "models":[], "created_at":0, "integration":"durindoor"
    })).unwrap());
    app.credentials.lock().unwrap().save(&app.config).unwrap();
    drop(app);
    std::fs::write(home.path().join("mcp.json"), r#"{"mcpServers":{"PRIVATE_MARKER":{"type":"PRIVATE_MARKER","command":"PRIVATE_MARKER","env":{"PRIVATE_MARKER":"PRIVATE_MARKER"}}}}"#).unwrap();
    for json_output in [true, false] {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_beans"));
        command.env_clear().env("BEANS_RELAY_URL", &url).env("RUST_LOG", "off");
        if let Some(root) = std::env::var_os("SystemRoot") { command.env("SystemRoot", root); }
        command.arg("--home").arg(home.path()).arg("doctor");
        if json_output { command.arg("--json"); }
        let output = command.output().unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains("PRIVATE_MARKER"));
        assert!(!text.contains(home.path().to_str().unwrap()));
        if json_output {
            let report: Value = serde_json::from_str(&text).unwrap();
            assert_private(&report, home.path());
            assert_eq!(report["relay"]["reason"], "unreachable");
            assert_eq!(report["mcp_json"]["problems"], 1);
        } else {
            assert!(text.contains("unreachable"));
        }
    }
}
