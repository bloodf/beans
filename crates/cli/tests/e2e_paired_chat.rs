//! End to end, through every real process boundary: a real `beans-relay`, two real `beans serve`
//! processes (a Runner and a paired Device, each with its own home), the local websocket API the
//! apps use, and a local OpenAI-compatible model server. The Device pairs with the Runner, the
//! Runner connects the model server as a custom provider and creates a bot, the Device sends the
//! bot a message, and the bot's reply reaches the Device through the relay. The relay's database
//! and files must never hold the plaintext of either message.
//!
//! Needs the relay binary: `cargo build -p beans-relay`, then run with `BEANS_E2E_RELAY_BIN` set
//! (`pnpm run test:e2e` and `pnpm run test:rust` do both). It reaches no network beyond 127.0.0.1.
#![cfg(all(feature = "cli", unix))]

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::Message as Frame;

const MODEL: &str = "e2e-echo";
/// A secret in the Runner's environment, by a secret's name, as a user's login shell has one.
const RUNNER_SECRET: (&str, &str) = ("OPENAI_API_KEY", "sk-proj-e2eQ7wXv2LmN8pR4tY6uZ1aB3cD5fG9h");
const STEP: Duration = Duration::from_secs(60);

/// The model server: lists one model and answers every chat completion with `pong:` and the
/// last user text, streamed as Chat Completions chunks. Every request body is kept.
#[derive(Clone, Default)]
struct ModelServer {
    requests: Arc<Mutex<Vec<Value>>>,
}

fn last_user_text(body: &Value) -> String {
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let Some(last) = messages.iter().rev().find(|m| m["role"] == "user") else { return String::new() };
    match &last["content"] {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

async fn models() -> Json<Value> {
    Json(json!({ "object": "list", "data": [{ "id": MODEL, "object": "model", "owned_by": "e2e" }] }))
}

async fn completions(State(server): State<ModelServer>, Json(body): Json<Value>) -> Response {
    server.requests.lock().unwrap().push(body.clone());
    let text = format!("pong: {}", last_user_text(&body));
    let chunk = |delta: Value, finish: Value| json!({
        "id": "e2e", "object": "chat.completion.chunk", "created": 0, "model": MODEL,
        "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
    });
    if body["stream"] == true {
        let mut sse = String::new();
        for event in [
            chunk(json!({ "role": "assistant", "content": text }), Value::Null),
            {
                let mut last = chunk(json!({}), json!("stop"));
                last["usage"] = json!({ "prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5 });
                last
            },
        ] {
            sse.push_str(&format!("data: {event}\n\n"));
        }
        sse.push_str("data: [DONE]\n\n");
        return ([("content-type", "text/event-stream")], sse).into_response();
    }
    Json(json!({
        "id": "e2e", "object": "chat.completion", "created": 0, "model": MODEL,
        "choices": [{ "index": 0, "message": { "role": "assistant", "content": text }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5 },
    })).into_response()
}

async fn start_model_server() -> (String, ModelServer) {
    let server = ModelServer::default();
    let router = Router::new()
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(completions))
        .with_state(server.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (url, server)
}

/// A child process killed when the test ends, whatever way it ends.
struct Owned(Child);
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.0.start_kill();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn base_env(command: &mut Command, home: &Path) {
    command.env_clear();
    for name in ["PATH", "TMPDIR", "LANG", "SystemRoot"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command.env("HOME", home).env("RUST_LOG", "warn");
}

async fn start_relay(binary: &Path, dir: &Path) -> (String, Owned) {
    let port = free_port();
    let mut command = Command::new(binary);
    base_env(&mut command, dir);
    command
        .arg("--bind").arg(format!("127.0.0.1:{port}"))
        .arg("--db").arg(dir.join("relay.sqlite3"))
        .arg("--files-dir").arg(dir.join("relay-files"))
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::inherit())
        .kill_on_drop(true);
    let child = Owned(command.spawn().expect("the relay binary starts"));
    let url = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(2)).build().unwrap();
    timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(response) = client.get(format!("{url}/v1/health")).send().await {
                if response.status().is_success() { break; }
            }
            sleep(Duration::from_millis(50)).await;
        }
    }).await.expect("the relay became healthy");
    (url, child)
}

/// One `beans serve` and a websocket client to it, like the apps' own.
struct Beans {
    _child: Owned,
    socket: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    next: u64,
    events: Vec<Value>,
}

impl Beans {
    async fn start(home: &Path, relay: &str, name: &str) -> Self {
        Self::start_with(home, relay, name, &[]).await
    }

    /// As `start`, with `extra` in the process environment.
    async fn start_with(home: &Path, relay: &str, name: &str, extra: &[(&str, &str)]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_beans"));
        base_env(&mut command, home);
        command.envs(extra.iter().copied());
        command
            .env("BEANS_RELAY_URL", relay)
            .args(["serve", "--ready-stdout", "--port", "0", "--home"]).arg(home.join("beans"))
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = command.spawn().unwrap_or_else(|error| panic!("{name}: beans serve starts: {error}"));
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        timeout(STEP, stdout.read_line(&mut line)).await.unwrap_or_else(|_| panic!("{name}: ready in time")).unwrap();
        let ready: Value = serde_json::from_str(&line).unwrap_or_else(|_| panic!("{name}: ready line, got {line:?}"));
        assert_eq!(ready["event"], "ready", "{name}");
        let port = ready["port"].as_u64().unwrap();
        // Keep draining stdout so a chatty child never blocks on a full pipe.
        tokio::spawn(async move { let mut sink = String::new(); while stdout.read_line(&mut sink).await.map(|n| n > 0).unwrap_or(false) { sink.clear(); } });
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await
            .unwrap_or_else(|error| panic!("{name}: websocket: {error}"));
        Beans { _child: Owned(child), socket, next: 1, events: Vec::new() }
    }

    /// Stops the process and waits until it is gone, releasing its home for a restart.
    async fn stop(&mut self) {
        let _ = self._child.0.kill().await;
    }

    /// Calls one method and returns its result, keeping the events that arrive meanwhile.
    async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next;
        self.next += 1;
        self.socket.send(Frame::Text(json!({ "id": id, "method": method, "params": params }).to_string().into())).await.unwrap();
        let reply = timeout(STEP, async {
            loop {
                let frame = self.socket.next().await.expect("socket open").expect("frame");
                let Ok(text) = frame.to_text() else { continue };
                let value: Value = serde_json::from_str(text).unwrap();
                if value["id"] == id { break value; }
                if value.get("event").is_some() || value.get("method").is_some() { self.events.push(value); }
            }
        }).await.unwrap_or_else(|_| panic!("{method} answered in time"));
        match reply.get("error") {
            Some(error) if !error.is_null() => Err(error.to_string()),
            _ => Ok(reply["result"].clone()),
        }
    }

    /// Polls `bootstrap` until `found` picks something out of the snapshot.
    async fn wait_for<T>(&mut self, what: &str, mut found: impl FnMut(&Value) -> Option<T>) -> T {
        let deadline = tokio::time::Instant::now() + STEP;
        loop {
            let snapshot = self.call("bootstrap", json!({})).await.expect("bootstrap");
            if let Some(value) = found(&snapshot) { return value; }
            assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}; last snapshot: {}", truncate(&snapshot));
            sleep(Duration::from_millis(250)).await;
        }
    }
}

fn truncate(value: &Value) -> String {
    let text = value.to_string();
    if text.len() > 4000 { format!("{}…", &text[..4000]) } else { text }
}

fn find_chat<'a>(snapshot: &'a Value, chat_id: &str) -> Option<&'a Value> {
    snapshot["chats"].as_array()?.iter().find(|chat| chat["id"] == chat_id)
}

fn message_texts(chat: &Value) -> Vec<(String, String)> {
    chat["messages"].as_array().into_iter().flatten().map(|message| {
        let author = message["author"].as_str().map(str::to_string)
            .unwrap_or_else(|| message["author"]["kind"].as_str().or(message["author"]["type"].as_str()).unwrap_or("").to_string());
        let text = message["text"].as_str().or(message["body"]["text"].as_str()).unwrap_or("").to_string();
        (author, text)
    }).collect()
}

/// Every byte the relay keeps: its SQLite files and its file store.
fn relay_bytes(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(&path).unwrap() {
            let entry = entry.unwrap().path();
            if entry.is_dir() { pending.push(entry); } else if let Ok(bytes) = std::fs::read(&entry) { out.push((entry, bytes)); }
        }
    }
    out
}

/// A relay, a Runner with a new account, a Device paired with it, and the model server connected
/// as the Runner's custom provider with one bot and its DM, seen on both sides.
struct Account {
    scratch: tempfile::TempDir,
    relay: String,
    relay_dir: PathBuf,
    _relay_child: Owned,
    model: ModelServer,
    runner: Beans,
    device: Beans,
    runner_id: String,
    device_id: String,
    kind: String,
    chat_id: String,
}

async fn paired_account() -> Account {
    let relay_bin = PathBuf::from(std::env::var_os("BEANS_E2E_RELAY_BIN").expect("BEANS_E2E_RELAY_BIN names the beans-relay binary"));
    let scratch = tempfile::tempdir().unwrap();
    let relay_dir = scratch.path().join("relay");
    std::fs::create_dir_all(&relay_dir).unwrap();
    let (relay, relay_child) = start_relay(&relay_bin, &relay_dir).await;
    let (model_url, model) = start_model_server().await;

    // The Runner: a new account, with the relay the test started.
    let runner_home = scratch.path().join("runner");
    std::fs::create_dir_all(&runner_home).unwrap();
    let mut runner = Beans::start_with(&runner_home, &relay, "runner", &[RUNNER_SECRET]).await;
    let hello = runner.call("hello", json!({})).await.unwrap();
    assert_eq!(hello["has_identity"], false);
    let created = runner.call("identity.create", json!({ "device_name": "E2E Runner" })).await.unwrap();
    let runner_id = created["device_id"].as_str().unwrap().to_string();
    // A versioned backup phrase: `beans-v2` and the base32 groups of the master secret.
    let phrase: Vec<&str> = created["phrase"].as_array().map(|groups| groups.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    assert!(phrase.first() == Some(&"beans-v2") && phrase.len() > 2, "a versioned backup phrase");
    runner.wait_for("the Runner's relay connection", |s| (s["relay_connected"] == true).then_some(())).await;

    // The Device pairs with the pairing string the Runner shows.
    let pairing = runner.call("pair.start", json!({})).await.unwrap();
    let nonce = pairing["nonce"].as_str().unwrap().to_string();
    let pairing_string = pairing["pairing_string"].as_str().unwrap().to_string();
    assert!(pairing_string.starts_with("beans://pair?v=2&"), "{pairing_string}");
    let device_home = scratch.path().join("device");
    std::fs::create_dir_all(&device_home).unwrap();
    let mut device = Beans::start(&device_home, &relay, "device").await;
    let accepted = device.call("pair.accept", json!({ "pairing_string": pairing_string, "device_name": "E2E Device" })).await.unwrap();
    assert!(accepted["identity_id"].as_str().is_some(), "{accepted}");
    let device_id = accepted["id"].as_str().unwrap().to_string();
    assert_ne!(device_id, runner_id);
    timeout(STEP, async {
        loop {
            let status = runner.call("pair.status", json!({ "nonce": nonce })).await.unwrap();
            match status["state"].as_str() {
                Some("completed") => break,
                Some("failed") => panic!("pairing failed: {status}"),
                _ => sleep(Duration::from_millis(200)).await,
            }
        }
    }).await.expect("pairing completes on the Runner");
    // Each side lists the other.
    runner.wait_for("the Device in the Runner's roster", |s| s["devices"].as_array()?.iter().any(|d| d["id"] == device_id).then_some(())).await;
    device.wait_for("the Runner in the Device's roster", |s| s["devices"].as_array()?.iter().any(|d| d["id"] == runner_id).then_some(())).await;

    // The Runner connects the model server and makes a bot that uses it, with a DM.
    let connected = runner.call("providers.connect_custom", json!({
        "name": "E2E Model", "base_url": model_url, "api_key": "e2e-key", "models": [MODEL], "api": "chat-completions",
    })).await.unwrap();
    let kind = connected["kind"].as_str().unwrap().to_string();
    assert!(kind.starts_with("custom:"), "{connected}");
    let bot = runner.call("bots.create", json!({
        "name": "Echo", "runner_id": runner_id, "provider": kind, "model": MODEL, "description": "Replies to end-to-end checks.",
    })).await.unwrap();
    let bot_id = bot["bot"]["id"].as_str().unwrap().to_string();
    let dm = runner.call("chats.dm", json!({ "bot_id": bot_id })).await.unwrap();
    let chat_id = dm["chat"]["id"].as_str().unwrap().to_string();

    // The Device learns the bot, its DM and the account's provider through the relay.
    device.wait_for("the bot's DM on the Device", |s| find_chat(s, &chat_id).map(|_| ())).await;
    device.wait_for("the provider on the Device", |s| s["providers"].as_array()?.iter().any(|p| p["kind"] == kind).then_some(())).await;
    Account { scratch, relay, relay_dir, _relay_child: relay_child, model, runner, device, runner_id, device_id, kind, chat_id }
}

/// Sends a unique `ping …` from `from` into the chat.
async fn send_ping(from: &mut Beans, chat_id: &str) -> String {
    let ping = format!("ping {}", uuid::Uuid::new_v4().simple());
    from.call("chats.send", json!({ "chat_id": chat_id, "text": ping })).await.unwrap();
    ping
}

/// Waits until `on` shows the bot's `pong:` reply to `ping`.
async fn reply_to(on: &mut Beans, chat_id: &str, ping: &str) -> String {
    on.wait_for("the bot's reply", |s| {
        message_texts(find_chat(s, chat_id)?).into_iter().map(|(_, text)| text).find(|text| text.contains("pong:") && text.contains(ping))
    }).await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the beans-relay binary in BEANS_E2E_RELAY_BIN (pnpm run test:e2e builds and supplies it)"]
async fn paired_device_chats_with_a_runner_bot_through_the_relay() {
    let mut account = paired_account().await;
    let chat_id = account.chat_id.clone();
    // The Device sends; the Runner's bot answers with the model; the answer comes back.
    let ping = send_ping(&mut account.device, &chat_id).await;
    let reply = reply_to(&mut account.device, &chat_id, &ping).await;
    assert!(reply.starts_with("pong:"), "{reply}");
    // The Runner holds the same conversation.
    account.runner.wait_for("the reply on the Runner", |s| {
        let texts: Vec<String> = message_texts(find_chat(s, &chat_id)?).into_iter().map(|(_, text)| text).collect();
        (texts.iter().any(|t| t == &ping) && texts.iter().any(|t| t == &reply)).then_some(())
    }).await;
    // The model was asked with the Device's words, under the bot's model id.
    let requests = account.model.requests.lock().unwrap().clone();
    assert!(requests.iter().any(|r| r["model"] == MODEL && last_user_text(r).contains(&ping)), "model requests: {requests:?}");
    // A message from the Runner's own side reaches the Device too.
    let runner_ping = send_ping(&mut account.runner, &chat_id).await;
    let runner_reply = reply_to(&mut account.device, &chat_id, &runner_ping).await;
    assert!(runner_reply.contains(&runner_ping));

    // End-to-end encryption: the relay stored ciphertext only.
    let stored = relay_bytes(&account.relay_dir);
    assert!(!stored.is_empty());
    for (path, bytes) in &stored {
        for needle in [ping.as_bytes(), reply.as_bytes(), runner_ping.as_bytes(), b"Replies to end-to-end checks.".as_slice(), b"e2e-key".as_slice(), account.kind.as_bytes()] {
            assert!(!bytes.windows(needle.len()).any(|window| window == needle), "{} holds plaintext {:?}", path.display(), String::from_utf8_lossy(needle));
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the beans-relay binary in BEANS_E2E_RELAY_BIN (pnpm run test:e2e builds and supplies it)"]
async fn a_restarted_runner_keeps_its_account_history_and_answers_again() {
    let mut account = paired_account().await;
    let chat_id = account.chat_id.clone();
    let before = send_ping(&mut account.device, &chat_id).await;
    reply_to(&mut account.device, &chat_id, &before).await;
    // Stop the Runner and start it again on the same home.
    let runner_home = account.scratch.path().join("runner");
    account.runner.stop().await;
    account.runner = Beans::start(&runner_home, &account.relay, "restarted runner").await;
    let hello = account.runner.call("hello", json!({})).await.unwrap();
    assert_eq!(hello["has_identity"], true);
    assert_eq!(hello["device_id"], account.runner_id);
    account.runner.wait_for("the restarted Runner's relay connection", |s| (s["relay_connected"] == true).then_some(())).await;
    account.runner.wait_for("the history after restart", |s| message_texts(find_chat(s, &chat_id)?).into_iter().any(|(_, t)| t == before).then_some(())).await;
    // The Device's next message is answered by the restarted process.
    let after = send_ping(&mut account.device, &chat_id).await;
    let reply = reply_to(&mut account.device, &chat_id, &after).await;
    assert!(reply.contains(&after));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the beans-relay binary in BEANS_E2E_RELAY_BIN (pnpm run test:e2e builds and supplies it)"]
async fn unpairing_a_device_removes_it_everywhere_and_it_forgets_the_account() {
    let mut account = paired_account().await;
    let device_id = account.device_id.clone();
    account.runner.call("device.unpair", json!({ "id": device_id })).await.unwrap();
    account.runner.wait_for("the Device to leave the Runner's roster", |s| (!s["devices"].as_array()?.iter().any(|d| d["id"] == device_id)).then_some(())).await;
    // The relay refuses the revoked key; the Device forgets the account it no longer belongs to.
    account.device.wait_for("the unpaired Device to forget the account", |s| (s["has_identity"] == false).then_some(())).await;
    let hello = account.device.call("hello", json!({})).await.unwrap();
    assert_eq!(hello["has_identity"], false);
    // The Runner keeps working alone.
    let runner_id = account.runner_id.clone();
    account.runner.wait_for("the Runner still in its own roster", |s| s["devices"].as_array()?.iter().any(|d| d["id"] == runner_id).then_some(())).await;
}

/// Issue #553, its automated part: a secret the Runner knows, pasted into a chat on the paired
/// Device, reaches the model server only as `{{secret:NAME}}`, in no request and in no form
/// (as is, JSON-escaped, percent-encoded, base64), while the bot still answers.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the beans-relay binary in BEANS_E2E_RELAY_BIN (pnpm run test:e2e builds and supplies it)"]
async fn a_secret_pasted_on_a_device_reaches_the_model_only_as_a_placeholder() {
    use base64::Engine;
    let mut account = paired_account().await;
    let chat_id = account.chat_id.clone();
    let (name, value) = RUNNER_SECRET;
    let marker = uuid::Uuid::new_v4().simple().to_string();
    let text = format!("deploy {marker} with the key {value} please");
    account.device.call("chats.send", json!({ "chat_id": chat_id, "text": text })).await.unwrap();
    let reply = account.device.wait_for("the bot's reply", |s| {
        message_texts(find_chat(s, &chat_id)?).into_iter().map(|(_, text)| text).find(|text| text.starts_with("pong:") && text.contains(&marker))
    }).await;
    assert!(reply.contains(&marker), "{reply}");

    let requests = account.model.requests.lock().unwrap().clone();
    let asked = requests.iter().find(|r| last_user_text(r).contains(&marker)).expect("the model was asked");
    assert!(last_user_text(asked).contains(&format!("the key {{{{secret:{name}}}}} please")), "{}", last_user_text(asked));
    let sent = serde_json::to_string(&requests).unwrap();
    let escaped = serde_json::to_string(value).unwrap();
    let percent: String = value.bytes().map(|b| format!("%{b:02X}")).collect();
    let base64 = base64::engine::general_purpose::STANDARD.encode(value);
    for form in [value, escaped.trim_matches('"'), percent.as_str(), base64.as_str()] {
        assert!(!sent.contains(form), "the model server received the secret as {form}");
    }
    // And the relay never held it in plaintext either.
    for (path, bytes) in relay_bytes(&account.relay_dir) {
        assert!(!bytes.windows(value.len()).any(|window| window == value.as_bytes()), "{} holds the secret", path.display());
    }
}
