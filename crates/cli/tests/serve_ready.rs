#![cfg(feature = "cli")]

use std::process::Stdio;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

struct Home(std::path::PathBuf);

impl Home {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("lorca-ready-{}", uuid::Uuid::new_v4())))
    }

    fn serve(&self, port: u16) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lorca"));
        command.env_clear();
        // Windows loads its network stack from under %SystemRoot%: without the variable, binding a
        // socket fails with WSAEPROVIDERFAILEDINIT.
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
        command
            .env("RUST_LOG", "off")
            .args([
                "serve",
                "--ready-stdout",
                "--port",
                &port.to_string(),
                "--home",
            ])
            .arg(&self.0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn readiness_is_flushed_with_logs_disabled_and_the_websocket_is_ready() {
    let home = Home::new();
    let mut child = home.serve(0).spawn().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    timeout(Duration::from_secs(5), stdout.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    if line.is_empty() {
        let mut stderr = String::new();
        let _ = timeout(Duration::from_secs(5), child.stderr.take().unwrap().read_to_string(&mut stderr)).await;
        panic!("lorca serve exited before it was ready ({:?}): {stderr}", child.wait().await);
    }
    let ready: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["event"], "ready");
    let port = ready["port"].as_u64().unwrap();
    assert!(port > 0);
    assert!(child.try_wait().unwrap().is_none());

    let (mut socket, _) = timeout(
        Duration::from_secs(2),
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")),
    )
    .await
    .unwrap()
    .unwrap();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({ "id": 7, "method": "bootstrap", "params": {} })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let response = timeout(Duration::from_secs(2), async {
        loop {
            let frame = socket.next().await.unwrap().unwrap();
            if let Ok(text) = frame.to_text() {
                let value: Value = serde_json::from_str(text).unwrap();
                if value["id"] == 7 {
                    break value;
                }
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(response["result"]["has_identity"], false);
    child.kill().await.unwrap();
}

#[tokio::test]
async fn a_failed_bind_exits_without_announcing_readiness() {
    let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let home = Home::new();
    let output = timeout(
        Duration::from_secs(5),
        home.serve(occupied.local_addr().unwrap().port()).output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

fn mcp_command(home: &Home, args: &[&str]) -> std::process::Output {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_lorca"));
    command.env("RUST_LOG", "off").arg("--home").arg(&home.0).arg("--port").arg("0").arg("mcp").args(args);
    command.output().unwrap()
}

#[test]
fn mcp_add_flags_and_import_skip_existing_without_exposing_secrets() {
    let home = Home::new();
    let output = mcp_command(&home, &["add", "remote", "--transport", "http", "--header", "Authorization=Bearer private-token", "-H", "X-Test=value", "--description", "Remote tools", "--timeout", "42", "https://example.com/mcp"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private-token"));
    let output = mcp_command(&home, &["add", "local", "--transport", "stdio", "--env", "TOKEN=private-token", "-e", "MODE=test", "--", "does-not-run", "--flag"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let path = home.0.join("mcp.json");
    let saved: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(saved["mcpServers"]["remote"]["headers"]["Authorization"], "Bearer private-token");
    assert_eq!(saved["mcpServers"]["remote"]["timeout"], 42);
    assert_eq!(saved["mcpServers"]["local"]["env"]["TOKEN"], "private-token");
    assert_eq!(saved["mcpServers"]["local"]["args"][0], "--flag");

    let source = home.0.join("source.json");
    std::fs::write(&source, r#"{"mcpServers":{"remote":{"type":"http","url":"https://other.example/mcp"},"new":{"command":"new-command"}}}"#).unwrap();
    let path = source.to_str().unwrap();
    let output = mcp_command(&home, &["import", path]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Skipped remote"));
    let output = mcp_command(&home, &["import", path]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Skipped new"));
    let saved: Value = serde_json::from_slice(&std::fs::read(home.0.join("mcp.json")).unwrap()).unwrap();
    assert_eq!(saved["mcpServers"]["remote"]["url"], "https://example.com/mcp");
    assert_eq!(saved["mcpServers"]["new"]["command"], "new-command");
}

#[test]
fn mcp_list_fails_for_enabled_invalid_entries_but_not_disabled_ones() {
    let home = Home::new();
    std::fs::create_dir_all(&home.0).unwrap();
    std::fs::write(home.0.join("mcp.json"), r#"{"mcpServers":{"broken":{"command":""}}}"#).unwrap();
    let output = mcp_command(&home, &["list"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("broken:"));
    std::fs::write(home.0.join("mcp.json"), r#"{"mcpServers":{"broken":{"command":"","disabled":true}}}"#).unwrap();
    let output = mcp_command(&home, &["list"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("broken: off"));
}
