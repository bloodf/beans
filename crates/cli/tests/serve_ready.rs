#![cfg(feature = "cli")]

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Notify, OnceCell};
use tokio::time::timeout;

type Preflight = Result<Duration, String>;
static CLI_PREFLIGHT: OnceCell<Preflight> = OnceCell::const_new();

async fn warm_binary(once: &OnceCell<Preflight>, mut command: Command) -> Preflight {
    once.get_or_init(|| async {
        let started = std::time::Instant::now();
        let output = timeout(Duration::from_secs(30), command.kill_on_drop(true).output())
            .await.map_err(|_| "executable preflight timed out".to_string())?
            .map_err(|error| format!("executable preflight failed: {error}"))?;
        if !output.status.success() {
            return Err(format!("executable preflight exited {}", output.status));
        }
        Ok(started.elapsed())
    }).await.clone()
}

async fn prepare_cli() {
    let home = Home::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_beans"));
    command.env_clear().env("RUST_LOG", "off").env("HOME", &home.0).env("USERPROFILE", &home.0);
    if let Some(root) = std::env::var_os("SystemRoot") { command.env("SystemRoot", root); }
    command.arg("--home").arg(&home.0).arg("--help").stdin(Stdio::null());
    let elapsed = warm_binary(&CLI_PREFLIGHT, command).await.expect("CLI executable preflight succeeds");
    // macOS can validate a newly linked, large debug Mach-O before Rust main starts. This
    // cold measurement stays visible and precedes, rather than extends, readiness deadlines.
    eprintln!("CLI executable preflight (outside readiness deadline): {elapsed:?}");
}
struct Home(std::path::PathBuf);

impl Home {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("beans-ready-{}", uuid::Uuid::new_v4())))
    }

    fn serve(&self, port: u16) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_beans"));
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
    prepare_cli().await;
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
        panic!("beans serve exited before it was ready ({:?}): {stderr}", child.wait().await);
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
    prepare_cli().await;
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

fn mcp_cli(home: &Home, port: u16, args: &[&str]) -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_beans"));
    command.env_clear().env("HOME", &home.0).env("USERPROFILE", &home.0);
    if let Some(root) = std::env::var_os("SystemRoot") { command.env("SystemRoot", root); }
    command.env("BEANS_MODELS_FETCH", "0").env("BEANS_MARKETPLACE_FETCH", "0");
    command.env("RUST_LOG", "off").arg("--home").arg(&home.0).arg("--port").arg(port.to_string()).arg("mcp").args(args);
    command
}

fn mcp_command(home: &Home, args: &[&str]) -> std::process::Output {
    mcp_cli(home, 0, args).output().unwrap()
}

/// A remote MCP server whose handshake reports it entered, then answers only once released.
#[derive(Clone)]
struct Held {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

async fn held_mcp(axum::extract::State(held): axum::extract::State<Held>, axum::Json(message): axum::Json<Value>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(id) = message.get("id").cloned() else { return axum::http::StatusCode::ACCEPTED.into_response() };
    let result = match message["method"].as_str() {
        Some("initialize") => {
            held.entered.notify_one();
            held.release.notified().await;
            json!({ "protocolVersion": message["params"]["protocolVersion"], "capabilities": { "tools": {} }, "serverInfo": { "name": "held", "version": "1" } })
        }
        Some("tools/list") => json!({ "tools": [] }),
        _ => json!({}),
    };
    axum::Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

/// Forwards one CLI websocket to `beans serve`, releasing the held handshake once the CLI asks
/// serve to connect a server.
async fn relay(client: tokio::net::TcpStream, serve_port: u16, release: Arc<Notify>) {
    let Ok(client) = tokio_tungstenite::accept_async(client).await else { return };
    let Ok((serve, _)) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{serve_port}/ws")).await else { return };
    let (mut to_client, mut from_client) = client.split();
    let (mut to_serve, mut from_serve) = serve.split();
    let up = async {
        while let Some(Ok(message)) = from_client.next().await {
            let reconnect = message.to_text().ok().and_then(|text| serde_json::from_str::<Value>(text).ok()).is_some_and(|request| request["method"] == "mcp.reconnect");
            if to_serve.send(message).await.is_err() {
                break;
            }
            if reconnect {
                release.notify_one();
            }
        }
    };
    let down = async {
        while let Some(Ok(message)) = from_serve.next().await {
            if to_client.send(message).await.is_err() {
                break;
            }
        }
    };
    tokio::select! { _ = up => {}, _ = down => {} }
}

/// `beans mcp list` against a running serve whose healthy server is still in its handshake (as
/// right after `beans mcp import`) waits for it instead of reporting it not ready.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_mcp_list_waits_for_a_healthy_server_still_connecting() {
    prepare_cli().await;
    let home = Home::new();
    // Dropped first on any exit: aborts the MCP server, the proxy, and its relays.
    let mut tasks = tokio::task::JoinSet::new();
    let held = Held { entered: Arc::new(Notify::new()), release: Arc::new(Notify::new()) };
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let router = axum::Router::new().route("/mcp", axum::routing::post(held_mcp)).with_state(held.clone());
    tasks.spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    beans::config::Config { home: home.0.clone(), port: 0 }.ensure_home().unwrap();
    let servers = json!({ "held": { "type": "http", "url": format!("{url}/mcp") }, "off": { "type": "http", "url": format!("{url}/off"), "disabled": true } });
    std::fs::write(home.0.join("mcp.json"), json!({ "mcpServers": servers }).to_string()).unwrap();

    let mut serve = home.serve(0);
    serve.env("HOME", &home.0).env("USERPROFILE", &home.0).env("BEANS_MODELS_FETCH", "0").env("BEANS_MARKETPLACE_FETCH", "0");
    let mut child = serve.spawn().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    timeout(Duration::from_secs(10), stdout.read_line(&mut line)).await.unwrap().unwrap();
    let serve_port = serde_json::from_str::<Value>(&line).expect("beans serve announces readiness")["port"].as_u64().unwrap() as u16;

    let proxy = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let proxy_port = proxy.local_addr().unwrap().port();
    let release = held.release.clone();
    tasks.spawn(async move {
        let mut relays = tokio::task::JoinSet::new();
        while let Ok((stream, _)) = proxy.accept().await {
            relays.spawn(relay(stream, serve_port, release.clone()));
        }
    });

    // Serve's own first connection is under way, so the server is not ready yet.
    timeout(Duration::from_secs(60), held.entered.notified()).await.expect("beans serve starts the handshake");
    let list = timeout(Duration::from_secs(60), Command::from(mcp_cli(&home, proxy_port, &["list"])).kill_on_drop(true).output()).await.expect("list returns").unwrap();
    assert!(list.status.success(), "{}", String::from_utf8_lossy(&list.stderr));

    // An enabled entry that cannot run still needs attention through the live serve.
    let servers = json!({ "held": { "type": "http", "url": format!("{url}/mcp") }, "broken": { "command": "" } });
    std::fs::write(home.0.join("mcp.json"), json!({ "mcpServers": servers }).to_string()).unwrap();
    let reload = timeout(Duration::from_secs(60), Command::from(mcp_cli(&home, proxy_port, &["reload"])).kill_on_drop(true).output()).await.expect("reload returns").unwrap();
    assert!(!reload.status.success());
    child.kill().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_add_flags_and_import_skip_existing_without_exposing_secrets() {
    prepare_cli().await;
    let home = Home::new();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let remote_url = format!("http://127.0.0.1:{}/mcp", listener.local_addr().unwrap().port());
    let server = tokio::spawn(async move {
        axum::serve(listener, axum::Router::new().fallback(|| async { axum::http::StatusCode::NOT_FOUND })).await.unwrap();
    });
    let output = mcp_command(&home, &["add", "remote", "--transport", "http", "--header", "Authorization: Bearer private-token", "-H", "X-Test: value", "--description", "Remote tools", "--timeout", "42", &remote_url]);
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
    let output = mcp_command(&home, &["import", path]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let saved: Value = serde_json::from_slice(&std::fs::read(home.0.join("mcp.json")).unwrap()).unwrap();
    assert_eq!(saved["mcpServers"]["remote"]["url"], remote_url);
    assert_eq!(saved["mcpServers"]["new"]["command"], "new-command");
    server.abort();
}

#[tokio::test]
async fn mcp_list_fails_for_enabled_invalid_entries_but_not_disabled_ones() {
    prepare_cli().await;
    let home = Home::new();
    beans::config::Config { home: home.0.clone(), port: 0 }.ensure_home().unwrap();
    std::fs::write(home.0.join("mcp.json"), r#"{"mcpServers":{"broken":{"command":""}}}"#).unwrap();
    let output = mcp_command(&home, &["list"]);
    assert!(!output.status.success());
    std::fs::write(home.0.join("mcp.json"), r#"{"mcpServers":{"broken":{"command":"","disabled":true}}}"#).unwrap();
    let output = mcp_command(&home, &["list"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[tokio::test]
async fn bare_beans_lists_the_commands_and_starts_nothing() {
    prepare_cli().await;
    let home = Home::new();
    let output = Command::new(env!("CARGO_BIN_EXE_beans")).env("BEANS_HOME", &home.0).env("RUST_LOG", "off").stdin(Stdio::null()).output();
    let output = timeout(Duration::from_secs(10), output).await.expect("it returns rather than serving").unwrap();
    assert!(output.status.success());
    assert!(!home.0.exists(), "a help page makes no data folder");
}

#[cfg(unix)]
fn delayed_preflight_fixture(home: &Home, fail: bool) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(&home.0).unwrap();
    let script = home.0.join("cold-cli");
    let count = home.0.join("count");
    std::fs::write(&script, format!("#!/bin/sh\nprintf x >> \"$1\"\nsleep 0.2\nexit {}\n", if fail { 9 } else { 0 })).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    (script, count)
}

#[cfg(unix)]
#[tokio::test]
async fn cold_executable_preflight_finishes_once_before_concurrent_readiness_timers() {
    let home = Home::new();
    let (script, count) = delayed_preflight_fixture(&home, false);
    let once = OnceCell::new();
    let run = || async {
        let mut command = Command::new(&script);
        command.arg(&count);
        warm_binary(&once, command).await.unwrap();
        timeout(Duration::from_millis(50), Command::new("/usr/bin/true").output()).await.unwrap().unwrap()
    };
    let (a, b, c) = tokio::join!(run(), run(), run());
    assert!(a.status.success() && b.status.success() && c.status.success());
    assert_eq!(std::fs::read(&count).unwrap(), b"x");
}

#[cfg(unix)]
#[tokio::test]
async fn executable_preflight_failure_is_shared() {
    let home = Home::new();
    let (script, count) = delayed_preflight_fixture(&home, true);
    let once = OnceCell::new();
    let run = || async {
        let mut command = Command::new(&script);
        command.arg(&count);
        warm_binary(&once, command).await
    };
    let (a, b) = tokio::join!(run(), run());
    assert_eq!(a, b);
    assert!(a.unwrap_err().contains("exited"));
    assert_eq!(std::fs::read(&count).unwrap(), b"x");
}
