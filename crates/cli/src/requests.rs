//! Questions one Device asks a Runner through the relay: a `request` blob sealed to the Runner's
//! box key, answered with a `response` blob sealed back to the Device that asked. The verbs read
//! and write a bot's memory, which lives on the bot's Runner, so the desktop app and phone show and
//! edit it for a bot that runs on another machine. A turn's `job` / `job_result` pair works the
//! same way; this is the same idea for questions with an answer.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::app::App;
use crate::config::now_secs;
use crate::memory::MemoryStore;
use crate::model::{Request, Response};

// A signed relay request retains its receiving account across every await. Task-local
// context adds no lock and is checked by each existing lifecycle admission. Spawned
// children must carry their own explicit incarnation (Tokio does not inherit this scope).
#[derive(Clone, Copy)]
struct RequestAuthority { app: usize, incarnation: u64 }
tokio::task_local! { static REQUEST_AUTHORITY: RequestAuthority; }
impl RequestAuthority {
    fn capture(app: &App) -> Result<Self, String> {
        let guard = app.plugin_admission(None)?;
        Ok(Self { app: app as *const App as usize, incarnation: guard.get().0 })
    }
}
pub(crate) fn admission_matches(app: &App, incarnation: u64) -> bool {
    REQUEST_AUTHORITY.try_with(|authority| authority.app != app as *const App as usize || authority.incarnation == incarnation).unwrap_or(true)
}

/// How long a question may wait for its answer. A Runner that is online answers in a few
/// seconds; past this the relay is slow or the Runner just went away.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Asks `runner_id` to run `verb` and waits for the answer. Refuses up front when the Runner is
/// unknown, unreachable, or offline, so an editor never spins on a machine that is asleep.
pub async fn ask(app: &Arc<App>, runner_id: &str, verb: &str, body: Value) -> Result<Value, String> {
    ask_within(app, runner_id, verb, body, REQUEST_TIMEOUT).await
}

/// `ask` for a verb the Runner takes longer to answer, waiting up to `timeout`.
pub async fn ask_within(app: &Arc<App>, runner_id: &str, verb: &str, body: Value, timeout: Duration) -> Result<Value, String> {
    let runner = app.device(runner_id).ok_or("That bot is assigned to a Runner this Device does not know yet.")?;
    if runner.box_pubkey.is_empty() {
        return Err(format!("{} has not shared its key yet.", runner.name));
    }
    if app.relay_url().is_none() {
        return Err(format!("No relay is configured, so this Device cannot reach {}.", runner.name));
    }
    if !app.device_is_online(runner_id) {
        return Err(format!("{} is offline.", runner.name));
    }
    let mut request = Request {
        id: format!("req-{}", uuid::Uuid::new_v4()),
        verb: verb.to_string(),
        requested_by: app.this_device_id().unwrap_or_default(),
        body,
        created_at: now_secs(),
    };
    let machine = app.machine_file().ok_or("Pair this Device first")?.machine().map_err(|_| "Device signing key unavailable")?;
    let payload = request_bytes(&request, runner_id, &request.body)?;
    request.body = json!({"payload": request.body, "signature": machine.sign(&payload)});
    let ciphertext = crate::crypto::seal_json(&runner.box_pubkey, &request).map_err(|e| e.to_string())?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.pending_responses.lock().unwrap().insert(request.id.clone(), tx);
    app.push_blob("request", Some(runner.id.clone()), ciphertext);
    let answer = tokio::time::timeout(timeout, rx).await;
    app.pending_responses.lock().unwrap().remove(&request.id);
    match answer {
        Ok(Ok(Response { error: Some(error), .. })) => Err(error),
        Ok(Ok(Response { body, .. })) => Ok(body),
        Ok(Err(_)) | Err(_) => Err(format!("{} did not answer in time.", runner.name)),
    }
}

/// A `response` reached this Device: hands it to the waiting `ask`, then drops the blob from
/// the relay.
pub fn deliver(app: Arc<App>, response: Response, blob_id: String) {
    if let Some(tx) = app.pending_responses.lock().unwrap().remove(&response.request_id) {
        let _ = tx.send(response);
    }
    if let Some(cleanup) = crate::sync::BlobCleanup::capture(&app) {
        tokio::spawn(async move { cleanup.delete(&app, &blob_id).await });
    }
}

/// A `request` reached this Runner: answers it, seals the answer to the Device that asked, and
/// drops the request from the relay. `running` counts it as work here until it is answered.
pub fn serve(app: Arc<App>, request: Request, blob_id: String, running: crate::update_control::Admission) {
    // Called while sync retains the originating account guard, before spawning.
    let Ok(authority) = RequestAuthority::capture(&app) else { return };
    let cleanup = crate::sync::BlobCleanup::capture(&app);
    tokio::spawn(REQUEST_AUTHORITY.scope(authority, async move {
        let _running = running;
        let result = answer_verified(&app, &request).await;
        if publish_answer(&app, &request, result).is_ok() {
            if let Some(cleanup) = cleanup { cleanup.delete(&app, &blob_id).await; }
        }
    }));
}

fn publish_answer(app: &Arc<App>, request: &Request, result: Result<Value, String>) -> Result<(), String> {
    let _guard = app.plugin_admission(None)?;
    let (body, error) = match result { Ok(body) => (body, None), Err(error) => (Value::Null, Some(error)) };
    let response = Response { request_id: request.id.clone(), body, error };
    let requester = app.device(&request.requested_by).filter(|d| !d.box_pubkey.is_empty()).ok_or("Unknown requester")?;
    if !app.state.lock().unwrap().listed_machines.contains_key(&request.requested_by) { return Err("Request requires a paired Device".into()); }
    let ciphertext = crate::crypto::seal_json(&requester.box_pubkey, &response).map_err(|e| e.to_string())?;
    app.push_blob("response", Some(requester.id), ciphertext);
    Ok(())
}

/// What this Runner can be asked. The memory verbs check that the bot runs here: a request
/// that reached the wrong machine is refused, not forwarded. The plugin verbs act on this
/// Runner's own installs, and the `mcp.*` verbs on its mcp.json; the permission verb answers a
/// card a bot here is waiting on; the bash verbs type into, stop, or background a command here;
/// Send now has a turn here read a message it holds. The separate self_update verbs
/// remain fail-closed in Beans; update.status/prepare/cancel belong to local drain control.
fn request_bytes(request: &Request, target: &str, body: &Value) -> Result<Vec<u8>, String> {
    // Keep the existing provider wire contract; other verbs use a separate domain.
    let domain = if matches!(request.verb.as_str(), "providers.custom.preview" | "providers.custom.save" | "providers.custom.refresh") {
        "beans-provider-request-v1"
    } else {
        "beans-request-v1"
    };
    serde_json::to_vec(&(domain, &request.id, &request.verb, &request.requested_by, target, request.created_at, body)).map_err(|_| "Invalid signed request".into())
}

#[cfg(test)]
async fn answer(app: &Arc<App>, request: &Request) -> Result<Value, String> {
    let authority = RequestAuthority::capture(app)?;
    REQUEST_AUTHORITY.scope(authority, answer_verified(app, request)).await
}

fn verify_request<'a>(app: &App, request: &'a Request) -> Result<&'a Value, String> {
    let _guard = app.plugin_admission(None)?;
    // Sealed boxes hide the payload but do not authenticate its claimed sender.
    // Check current membership and signing authority before dispatching any verb.
    let paired = app.state.lock().unwrap().listed_machines.contains_key(&request.requested_by);
    if !paired { return Err("Request requires a paired Device".into()); }
    let target = app.this_device_id().ok_or("Pair a Runner first")?;
    let body = request.body.get("payload").ok_or("Unsigned request")?;
    let signature = request.body["signature"].as_str().ok_or("Unsigned request")?;
    let key = crate::keys::verifying_key(&request.requested_by).map_err(|_| "Invalid requester signing key")?;
    let bytes = crate::keys::unb64(signature).map_err(|_| "Invalid request signature")?;
    let signature = ed25519_dalek::Signature::from_slice(&bytes).map_err(|_| "Invalid request signature")?;
    key.verify_strict(&request_bytes(request, &target, body)?, &signature).map_err(|_| "Invalid request signature")?;
    Ok(body)
}

async fn answer_verified(app: &Arc<App>, request: &Request) -> Result<Value, String> {
    let body = verify_request(app, request)?;
    dispatch_verified(app, request, body).await
}

// The shell handler does not acquire roster_edit: serialize one synchronous poll with
// reset, then release before Pending. Do not use this wrapper for roster writers.
#[cfg(feature = "runner")]
async fn poll_account_bound<F: std::future::Future<Output = Result<Value, String>>>(app: &App, future: F) -> Result<Value, String> {
    let mut future = std::pin::pin!(future);
    std::future::poll_fn(|cx| {
        let _guard = match app.plugin_admission(None) { Ok(guard) => guard, Err(error) => return std::task::Poll::Ready(Err(error)) };
        future.as_mut().poll(cx)
    }).await
}

async fn dispatch_verified(app: &Arc<App>, request: &Request, body: &Value) -> Result<Value, String> {
    match request.verb.as_str() {
        "skills.discovery" => crate::skill_discovery::serve_as(app, body.clone(), &request.requested_by).await,
        "skills.library.preview" | "skills.library.import" | "skills.library.uninstall" => crate::skill_library::serve_as(app, &request.verb, body.clone(), &request.requested_by).await,
        "skills.library.compare" => crate::skill_library::serve_as(app, &request.verb, body.clone(), &request.requested_by).await,
        #[cfg(feature = "provider-auth")]
        "providers.custom.preview" | "providers.custom.save" | "providers.custom.refresh" => {
            let requester = app.device(&request.requested_by).filter(|device| !device.box_pubkey.is_empty()).ok_or("Provider request requires a paired Device")?;
            crate::provider_auth::serve_guided_as(app, &request.verb, body, Some((&requester.id, &requester.box_pubkey))).await
        }
        verb if crate::memory_service::api::is_setup_method(verb) => {
            crate::memory_service::api::serve_as(app,verb,body.clone(),&request.requested_by).await
        },
        verb if verb.starts_with("memory.service.") || verb.starts_with("memory.operations.") =>
            crate::memory_service::api::serve(app, verb, body.clone()).await,
        "memory.read" => memory_read(app, body["bot_id"].as_str().ok_or("missing bot_id")?),
        "memory.write" => {
            let text = body["text"].as_str().ok_or("missing text")?;
            memory_write(app, body["bot_id"].as_str().ok_or("missing bot_id")?, text, body["expected_hash"].as_str())
        }
        #[cfg(feature = "runner")]
        verb if verb.starts_with("mcp.") && verb != "mcp.parse" => crate::plugins::mcp_json::serve_request(app, verb, body).await,
        #[cfg(feature = "runner")]
        verb if verb.starts_with("plugins.") || verb == "permission.answer" => crate::plugins::serve_request(app, verb, body, Some(&request.requested_by)).await,
        #[cfg(feature = "runner")]
        verb if verb.starts_with("mcp.") => crate::plugins::mcp_json::serve_request(app, verb, body).await,
        #[cfg(feature = "runner")]
        "bash.stdin" | "bash.stop" | "bash.background" => poll_account_bound(app, crate::shell::serve(app, &request.verb, body)).await,
        #[cfg(feature = "runner")]
        "chats.send_now" => {
            let _guard = app.plugin_admission(None)?;
            let chat_id = body["chat_id"].as_str().ok_or("missing chat_id")?;
            let message_id = body["message_id"].as_str().ok_or("missing message_id")?;
            crate::turns::send_now(app, chat_id, message_id).map(|sent| json!({ "sent": sent }))
        }
        #[cfg(feature = "cli")]
        "self_update.install" => crate::update::install_now(app).await,
        #[cfg(feature = "cli")]
        "self_update.auto" => crate::update::set_auto(app, body["on"].as_bool().ok_or("missing on")?),
        other => Err(format!("Unknown request {other}")),
    }
}

#[cfg(all(test, feature = "runner", target_os = "linux"))]
#[path = "skill_discovery_test.rs"]
mod skill_discovery_tests;

#[cfg(all(test, feature = "runner", target_os = "linux"))]
#[path = "skill_library_test.rs"]
mod skill_library_tests;

#[cfg(all(test, feature = "runner", target_os = "linux"))]
#[test]
fn signed_library_compare_is_readonly_and_root_bounded() {
    use std::future::Future;
    use std::task::Poll;
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().max_blocking_threads(1).build().unwrap();
    runtime.block_on(async {
        let home = std::env::temp_dir().join(format!("beans-library-compare-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&home).unwrap();
        let _cleanup = Cleanup(home.clone());
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        crate::identity::create(&app, Some("Synthetic Runner".into())).unwrap();
        let runner = app.this_device_id().unwrap();
        let phone = crate::keys::Machine::generate();
        {
            let mut state = app.state.lock().unwrap();
            state.devices.push(crate::model::Device { id: phone.pubkey(), name: "Synthetic phone".into(), os: "ios".into(), box_pubkey: phone.box_pubkey(), ..Default::default() });
            state.listed_machines.insert(phone.pubkey(), 1);
            state.listed_machines.insert(runner.clone(), 1);
            state.devices.iter_mut().find(|d| d.id == runner).unwrap().os = "linux".into();
        }
        let source = home.join("synthetic-source");
        std::fs::create_dir_all(source.join("resources")).unwrap();
        let skill = b"---\nname: Compare fixture\ndescription: Inactive copy\n---\nNEVER_EXECUTE_OR_RETURN_BODY\n";
        std::fs::write(source.join("SKILL.md"), skill).unwrap();
        for (path, bytes) in [("changed", b"old"), ("removed", b"old"), ("same", b"old")] {
            std::fs::write(source.join("resources").join(path), bytes).unwrap();
        }
        let make = |verb: &str, body: Value| {
            let mut request = Request { id: format!("synthetic-{}", uuid::Uuid::new_v4()), verb: verb.into(), requested_by: phone.pubkey(), body: body.clone(), created_at: now_secs() };
            request.body = json!({"payload":body,"signature":phone.sign(&request_bytes(&request, &runner, &body).unwrap())});
            request
        };
        let compare_body = |id: &str| json!({"version":1,"runner_id":runner,"root":source,"managed_id":id});
        assert_eq!(answer(&app, &make("skills.library.compare", compare_body(&"0".repeat(32)))).await.unwrap_err(), "skill_library_failed");
        assert!(!home.join("skill-library").exists(), "Compare must not create a library");
        let preview = answer(&app, &make("skills.library.preview", json!({"version":1,"runner_id":runner,"root":source}))).await.unwrap();
        let imported = answer(&app, &make("skills.library.import", json!({"version":1,"runner_id":runner,"token":preview["token"],"confirmed":true}))).await.unwrap();
        let id = imported["managed_id"].as_str().unwrap();
        let owned = home.join("skill-library");
        let bundle = owned.join(id);
        let manifest = std::fs::read(bundle.join("manifest.json")).unwrap();
        let expected = json!({"version":1,"runner_id":runner,"managed_id":id,"added":[],"removed":[],"changed":[]});
        assert_eq!(answer(&app, &make("skills.library.compare", compare_body(id))).await.unwrap(), expected);
        std::fs::write(source.join("resources/changed"), b"new").unwrap();
        std::fs::remove_file(source.join("resources/removed")).unwrap();
        std::fs::write(source.join("resources/added"), b"new").unwrap();
        let expected = json!({"version":1,"runner_id":runner,"managed_id":id,"added":["resources/added"],"removed":["resources/removed"],"changed":["resources/changed"]});
        let request = make("skills.library.compare", compare_body(id));
        assert_eq!(answer(&app, &request).await.unwrap(), expected);
        assert_eq!(crate::api::dispatch(&app, "skills.library.compare", compare_body(id)).await.unwrap(), expected);
        let mut forged = request.clone();
        forged.body["payload"]["managed_id"] = json!("0".repeat(32));
        assert_eq!(answer(&app, &forged).await.unwrap_err(), "Invalid request signature");
        let mut wrong_runner = compare_body(id);
        wrong_runner["runner_id"] = json!(phone.pubkey());
        assert_eq!(answer(&app, &make("skills.library.compare", wrong_runner)).await.unwrap_err(), "skill_library_authority_changed");
        let mut invalid = compare_body(id);
        invalid["managed_id"] = json!("../source");
        assert_eq!(answer(&app, &make("skills.library.compare", invalid)).await.unwrap_err(), "invalid_skill_library_request");
        std::os::unix::fs::symlink(source.join("resources/same"), source.join("resources/link")).unwrap();
        assert_eq!(answer(&app, &request).await.unwrap_err(), "skill_library_failed");
        std::fs::remove_file(source.join("resources/link")).unwrap();
        std::fs::remove_file(bundle.join("content/resources/same")).unwrap();
        std::os::unix::fs::symlink(source.join("resources/same"), bundle.join("content/resources/same")).unwrap();
        assert_eq!(answer(&app, &request).await.unwrap_err(), "skill_library_failed");
        std::fs::remove_file(bundle.join("content/resources/same")).unwrap();
        std::fs::write(bundle.join("content/resources/same"), b"old").unwrap();
        // Revoke actor while compare waits for its bounded blocking read.
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || { started_tx.send(()).unwrap(); let _ = release_rx.recv(); });
        started_rx.recv().unwrap();
        let mut pending = Box::pin(answer(&app, &request));
        std::future::poll_fn(|cx| { assert!(matches!(pending.as_mut().poll(cx), Poll::Pending)); Poll::Ready(()) }).await;
        app.state.lock().unwrap().listed_machines.remove(&phone.pubkey());
        release_tx.send(()).unwrap();
        assert_eq!(pending.await.unwrap_err(), "skill_library_authority_changed");
        blocker.await.unwrap();
        assert_eq!(answer(&app, &request).await.unwrap_err(), "Request requires a paired Device");
        assert_eq!(std::fs::read(bundle.join("manifest.json")).unwrap(), manifest);
        assert_eq!(std::fs::read(bundle.join("content/SKILL.md")).unwrap(), skill);
        for path in ["changed", "removed", "same"] {
            assert_eq!(std::fs::read(bundle.join("content/resources").join(path)).unwrap(), b"old");
        }
        assert!(!bundle.join("content/resources/added").exists());
        assert_eq!(std::fs::read_dir(&owned).unwrap().map(|entry| entry.unwrap().file_name()).collect::<Vec<_>>(), vec![std::ffi::OsString::from(id)]);
    });
}

/// A bot's memory as this Runner has it: the index with its budget and the other files by name.
pub fn memory_read(app: &Arc<App>, bot_id: &str) -> Result<Value, String> {
    let _guard = app.plugin_admission(None)?;
    let bot = local_bot(app, bot_id)?;
    Ok(MemoryStore::for_bot(&app.config.home, &bot).overview())
}

/// Replaces a bot's MEMORY.md on this Runner, refusing when it changed since `expected_hash`.
pub fn memory_write(app: &Arc<App>, bot_id: &str, text: &str, expected_hash: Option<&str>) -> Result<Value, String> {
    let _guard = app.plugin_admission(None)?;
    let bot = local_bot(app, bot_id)?;
    let hash = MemoryStore::for_bot(&app.config.home, &bot).write_index(text, expected_hash).map_err(|e| e.to_string())?;
    Ok(json!({ "hash": hash }))
}

fn local_bot(app: &Arc<App>, bot_id: &str) -> Result<crate::model::Bot, String> {
    let bot = app.bot(bot_id).ok_or("Unknown bot")?;
    if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
        let runner = app.device(&bot.runner_id).map(|d| d.name).unwrap_or_else(|| "another Runner".into());
        return Err(format!("{} runs on {runner}, not here.", bot.name));
    }
    Ok(bot)
}

#[cfg(all(test, feature = "provider-auth"))]
mod custom_provider_tests {
    use super::*;

    #[tokio::test]
    async fn phone_request_executes_catalog_http_only_on_selected_runner() {
        use std::io::{Read, Write};
        let home = std::env::temp_dir().join(format!("beans-guided-origin-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&home).unwrap();
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        crate::identity::create(&app, Some("Fixture Runner".into())).unwrap();
        let runner = app.this_device_id().unwrap();
        let phone = crate::keys::Machine::generate();
        let unrelated = crate::keys::Machine::generate();
        app.state.lock().unwrap().devices.push(crate::model::Device { id: phone.pubkey(), name: "Fixture phone".into(), os: "ios".into(), box_pubkey: phone.box_pubkey(), ..Default::default() });
        app.state.lock().unwrap().listed_machines.insert(phone.pubkey(), 1);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let root = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, peer) = listener.accept().unwrap();
            assert!(peer.ip().is_loopback());
            let mut bytes = [0; 4096];
            let size = socket.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..size]).into_owned();
            let body = r#"{"data":[{"id":"runner-local-alias"}]}"#;
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            request
        });
        let mut request = Request { id: "fixture-preview".into(), verb: "providers.custom.preview".into(), requested_by: "foreign".into(), body: json!({"runner_id":runner,"api":"chat-completions","base_url":root}), created_at: now_secs() };
        assert!(answer(&app, &request).await.unwrap_err().contains("paired Device"));
        request.requested_by = phone.pubkey();
        let unsigned = request.clone();
        let signature = unrelated.sign(&request_bytes(&unsigned, &runner, &unsigned.body).unwrap());
        request.body = json!({"payload": unsigned.body, "signature": signature});
        let runner_keys = app.machine_file().unwrap().machine().unwrap();
        let forged = crate::crypto::seal_json(&runner_keys.box_pubkey(), &request).unwrap();
        let decoded: Request = crate::crypto::unseal_json(&runner_keys.box_secret, &forged).unwrap();
        assert!(answer(&app, &decoded).await.unwrap_err().contains("Invalid request signature"));
        assert!(answer(&app, &request).await.unwrap_err().contains("Invalid request signature"));
        let signature = phone.sign(&request_bytes(&unsigned, &runner, &unsigned.body).unwrap());
        request.body = json!({"payload": unsigned.body, "signature": signature});
        app.state.lock().unwrap().devices.retain(|device| device.id != phone.pubkey());
        app.state.lock().unwrap().listed_machines.remove(&phone.pubkey());
        assert!(answer(&app, &request).await.unwrap_err().contains("paired Device"));
        assert!(app.credentials.lock().unwrap().custom.is_empty());
        app.state.lock().unwrap().devices.push(crate::model::Device { id: phone.pubkey(), name: "Fixture phone".into(), os: "ios".into(), box_pubkey: phone.box_pubkey(), ..Default::default() });
        app.state.lock().unwrap().listed_machines.insert(phone.pubkey(), 1);
        let result = answer(&app, &request).await.unwrap();
        assert_eq!(result["models"][0]["id"], "runner-local-alias");
        assert!(server.join().unwrap().starts_with("GET /v1/models "));
        assert!(app.credentials.lock().unwrap().custom.is_empty());
        drop(app);
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[cfg(all(test, feature = "runner"))]
mod account_authority_tests {
    use super::*;

    #[test]
    fn plugin_and_mcp_rule_cleanup_preserves_policy_without_recursive_roster_lock() {
        let home=tempfile::tempdir().unwrap();
        let app=App::load(crate::config::Config{home:home.path().to_owned(),port:0}).unwrap();
        crate::identity::create(&app,Some("Policy cleanup".into())).unwrap();
        let rule=|id:&str|crate::model::AutoReviewRule{id:id.into(),text:"fixture".into(),behavior:"allow".into(),tool:Some(format!("{id}/read"))};
        let manifest=crate::plugins::Manifest::parse(&json!({"id":"policy-fixture","name":"Policy fixture","servers":{"api":{"type":"http","url":"http://127.0.0.1:1/mcp"}}})).unwrap();
        crate::plugins::install(&app,manifest,"inline").unwrap();
        app.add_auto_review_rule(rule("policy-fixture"));
        crate::plugins::uninstall(&app,"policy-fixture").unwrap();
        assert!(app.auto_review().rules.is_empty());
        let config=json!({"command":"synthetic-no-execution"});
        crate::plugins::mcp_json::save(&app,"before",None,&config).unwrap();
        app.add_auto_review_rule(rule(&crate::plugins::mcp_json::server_id("before")));
        crate::plugins::mcp_json::save(&app,"after",Some("before"),&config).unwrap();
        assert_eq!(app.auto_review().rules[0].tool,Some(format!("{}/read",crate::plugins::mcp_json::server_id("after"))));
        crate::plugins::mcp_json::remove(&app,"after").unwrap();
        assert!(app.auto_review().rules.is_empty());
    }

    #[tokio::test]
    async fn verified_request_cannot_dispatch_or_publish_after_account_reset() {
        let home = tempfile::tempdir().unwrap();
        let app = App::load(crate::config::Config { home: home.path().to_owned(), port: 0 }).unwrap();
        crate::identity::create(&app, Some("Request A".into())).unwrap();
        let phone = crate::keys::Machine::generate();
        let runner = app.this_device_id().unwrap();
        app.state.lock().unwrap().listed_machines.insert(phone.pubkey(), 1);
        app.state.lock().unwrap().devices.push(crate::model::Device { id: phone.pubkey(), box_pubkey: phone.box_pubkey(), ..Default::default() });
        let payload = json!({"manifest":{"id":"old-request","name":"Old account plugin","servers":{"api":{"type":"http","url":"http://127.0.0.1:1/mcp"}}}});
        let mut request = Request { id: "reset-request".into(), verb: "plugins.install".into(), requested_by: phone.pubkey(), body: payload.clone(), created_at: now_secs() };
        request.body = json!({"payload":payload,"signature":phone.sign(&request_bytes(&request,&runner,&payload).unwrap())});
        // Positive control: exactly this signed request is valid and installs before reset.
        assert!(answer(&app,&request).await.is_ok());
        assert!(app.plugins.lock().unwrap().get("old-request").is_some());
        let authority = RequestAuthority::capture(&app).unwrap();
        let (verified_tx, verified_rx) = tokio::sync::oneshot::channel();
        let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
        let pending = tokio::spawn({ let app=app.clone(); async move {
            REQUEST_AUTHORITY.scope(authority, async {
                let body = verify_request(&app,&request).unwrap().clone();
                verified_tx.send(()).unwrap();
                resume_rx.await.unwrap();
                let result = dispatch_verified(&app,&request,&body).await;
                assert!(result.is_err(), "verified A request must not install into B");
                assert!(publish_answer(&app,&request,Ok(json!({"old":"response"}))).is_err());
                assert!(verify_request(&app,&request).is_err());
                let replacement_bot = app.state.lock().unwrap().bots[0].id.clone();
                assert!(memory_read(&app,&replacement_bot).is_err());
                assert!(memory_write(&app,&replacement_bot,"old account text",None).is_err());
            }).await;
        }});
        verified_rx.await.unwrap();
        app.forget_identity().unwrap();
        crate::identity::create(&app,Some("Request B".into())).unwrap();
        // Even re-pairing the same sender cannot reauthorize A's already-verified call.
        app.state.lock().unwrap().listed_machines.insert(phone.pubkey(),1);
        app.state.lock().unwrap().devices.push(crate::model::Device { id:phone.pubkey(),box_pubkey:phone.box_pubkey(),..Default::default() });
        let outbox = app.store.outbox().unwrap().len();
        resume_tx.send(()).unwrap();
        pending.await.unwrap();
        assert!(app.plugins.lock().unwrap().get("old-request").is_none());
        assert_eq!(app.store.outbox().unwrap().len(),outbox,"stale response must not enter B outbox");
        assert!(app.plugin_admission(None).is_ok(),"request scope must not leak into caller");
    }
}
