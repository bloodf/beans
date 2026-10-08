use beans::{app::App, config::Config, diagnostics, plugins::Store};

#[tokio::test]
async fn malformed_mcp_and_unknown_cached_state_do_not_echo_private_text() {
    let home = tempfile::tempdir().unwrap();
    let app = App::load(Config { home: home.path().to_owned(), port: 0 }).unwrap();
    // Invalid URL prevents all network traffic, independently of inherited relay settings.
    app.settings.lock().unwrap().relay_url = Some("PRIVATE_MARKER".into());
    for contents in [
        "PRIVATE_MARKER{",
        r#"{"mcpServers":{"PRIVATE_MARKER":{"command":"PRIVATE_MARKER","cwd":"PRIVATE_MARKER","env":{"PRIVATE_MARKER":9}}}}"#,
        r#"{"mcpServers":{"PRIVATE_MARKER":{"url":"http://PRIVATE_MARKER","headers":{"PRIVATE_MARKER":9}}}}"#,
        r#"{"mcpServers":{"PRIVATE_MARKER":{"command":"PRIVATE_MARKER","toolExposure":{"mode":"selected","tools":["PRIVATE_MARKER["]}}}}"#,
        r#"{"mcpServers":{"PRIVATE_MARKER":{"command":"PRIVATE_MARKER","env":{"PRIVATE_MARKER":"PRIVATE_MARKER"}}}}"#,
    ] {
        std::fs::write(home.path().join("mcp.json"), contents).unwrap();
        let mut store = Store::load(&app.config).unwrap();
        for status in store.statuses() {
            store.notes.insert(status.id, ("PRIVATE_MARKER".into(), "PRIVATE_MARKER".into()));
        }
        *app.plugins.lock().unwrap() = store;
        let report = diagnostics::report(&app).await;
        assert!(!report.to_string().contains("PRIVATE_MARKER"));
        assert_eq!(report["relay"]["reason"], "invalid_url");
        assert_eq!(report["relay"]["reachable"], serde_json::Value::Null);
        if contents == "PRIVATE_MARKER{" {
            assert_eq!(report["mcp_json"]["file_error"], true);
        }
        for entry in report["plugins"]["entries"].as_array().unwrap() {
            assert_eq!(entry["state"], "unknown");
        }
    }
}
