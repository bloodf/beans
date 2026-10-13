#![cfg(feature = "server")]

use std::{io::Write, process::Stdio, sync::{Arc, atomic::{AtomicUsize, Ordering}}, time::Duration};

use axum::{extract::{Query, Request, State, ws::WebSocketUpgrade}, http::{Method, StatusCode}, middleware::{self, Next}, response::{IntoResponse, Response}, routing::{get, post}, Json, Router};
use beans::{app::App, config::Config, model::{Bot, Capabilities, Chat, ChatMeta, RosterBlob}};
use serde_json::{json, Value};
use tokio::{io::{AsyncBufReadExt, BufReader}, process::Command, time::timeout};

const CHECKPOINT: &str = "RECONNECT_CHECKPOINT ";
const CONFLICT: &str = "Roster conflict: group group would have 0 members (allowed 1–6). Local changes remain queued; reconcile the conflicting chat on a paired Device before retrying";

// Read every persisted envelope, including attachments, without test-only LocalStore exports.
fn persisted(app: &App) -> Value {
    let db = rusqlite::Connection::open_with_flags(
        app.config.home.join("beans.sqlite3"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).unwrap();
    let mut statement = db.prepare(
        "SELECT id, kind, recipient, ciphertext, slot_name, slot_keep_first, group_name FROM outbox ORDER BY position",
    ).unwrap();
    let envelopes = statement.query_map([], |row| {
        Ok(json!([row.get::<_, String>(0)?, row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?, row.get::<_, Vec<u8>>(3)?,
            row.get::<_, Option<String>>(4)?, row.get::<_, bool>(5)?,
            row.get::<_, Option<String>>(6)?]))
    }).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    json!({"outbox": envelopes, "baseline": app.store.roster_baseline("queued").unwrap(),
        "chat": app.chat("group").unwrap().meta})
}

// The integration-test executable is the child Runner-core process. It invokes the public
// production sync loop, not a copied algorithm or an in-process App reload in the parent.
#[tokio::test]
#[ignore = "child entry point invoked by the process-restart fixture"]
async fn reconnect_process_worker() {
    let home = std::env::var_os("BEANS_RECONNECT_FIXTURE_HOME").expect("parent supplies isolated home");
    let app = App::load(Config { home: home.into(), port: 0 }).unwrap();
    let mut events = app.events.subscribe();
    println!("{CHECKPOINT}{}", json!({"pid": std::process::id(), "persisted": persisted(&app)}));
    std::io::stdout().flush().unwrap();
    let task = tokio::spawn(beans::sync::run(app.clone()));
    loop {
        if let beans::events::Event::RelayStatus { connected: false, error: Some(problem), .. } = events.recv().await.unwrap() {
            println!("{CHECKPOINT}{}", json!({"pid": std::process::id(), "persisted": persisted(&app),
                "error": {"message": problem.message, "unknown_machine": problem.unknown_machine}}));
            std::io::stdout().flush().unwrap();
            app.outbox_notify.notify_one();
        }
        assert!(!task.is_finished(), "production sync loop exited");
    }
}

#[derive(Clone)]
struct Relay {
    roster: Value,
    upgrades: Arc<AtomicUsize>,
    writes: Arc<AtomicUsize>,
}

async fn list(State(relay): State<Relay>, Query(query): Query<std::collections::HashMap<String, String>>) -> Json<Value> {
    let blobs = if query["kinds"] == "roster" { vec![relay.roster] } else { vec![] };
    Json(json!({"blobs": blobs, "seq": 8}))
}

async fn upgrade(State(relay): State<Relay>, ws: WebSocketUpgrade) -> axum::response::Response {
    ws.on_upgrade(move |mut socket| async move {
        relay.upgrades.fetch_add(1, Ordering::Relaxed);
        while socket.recv().await.is_some() {}
    })
}

async fn reject_mutation(State(relay): State<Relay>, request: Request, next: Next) -> Response {
    let authentication = request.method() == Method::POST
        && matches!(request.uri().path(), "/v1/auth/challenge" | "/v1/auth/verify");
    if !matches!(*request.method(), Method::GET | Method::HEAD | Method::OPTIONS) && !authentication {
        relay.writes.fetch_add(1, Ordering::Relaxed);
        return StatusCode::CONFLICT.into_response();
    }
    next.run(request).await
}

fn relay_router(relay: Relay) -> Router {
    Router::new()
        .route("/v1/health", get(|| async { Json(json!({"ok": true, "service": "beans-relay", "format": "beans-v2",
            "protocol": 5, "min_protocol": 5, "min_roster_protocol": 5, "memory_config_version": 1})) }))
        .route("/v1/auth/challenge", post(|| async { Json(json!({"nonce": "synthetic-challenge"})) }))
        .route("/v1/auth/verify", post(|| async { Json(json!({"token": "synthetic-token"})) }))
        .route("/v1/sync", get(upgrade)).route("/v1/blobs", get(list))
        .with_state(relay.clone())
        // Router-wide middleware also sees unmatched paths and method-not-allowed requests.
        .layer(middleware::from_fn_with_state(relay, reject_mutation))
}

#[tokio::test]
async fn relay_observer_counts_and_rejects_file_and_unmatched_mutations() {
    let relay = Relay { roster: Value::Null, upgrades: Arc::new(AtomicUsize::new(0)), writes: Arc::new(AtomicUsize::new(0)) };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let router = relay_router(relay.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap();
    // Neither path has a route: old route-local observation returned an uncounted 404.
    let mutations = [
        (Method::PUT, "/v1/files/group-file"), (Method::PUT, "/unmatched"),
        (Method::PUT, "/v1/blobs"), (Method::PUT, "/v1/health"),
        (Method::DELETE, "/v1/blobs/remote-roster"), (Method::DELETE, "/v1/groups/group"),
        (Method::DELETE, "/v1/files/group-file"), (Method::DELETE, "/unmatched"),
        (Method::POST, "/unmatched"), (Method::PATCH, "/v1/files/group-file"),
    ];
    for (index, (method, path)) in mutations.into_iter().enumerate() {
        let response = client.request(method, format!("{url}{path}")).body("synthetic ciphertext").send().await.unwrap();
        assert_eq!(relay.writes.load(Ordering::Relaxed), index + 1, "mutation escaped ingress observation: {path}");
        assert_eq!(response.status(), StatusCode::CONFLICT, "mutation reached route rejection: {path}");
    }
    assert_eq!(client.get(format!("{url}/unmatched")).send().await.unwrap().status(), StatusCode::NOT_FOUND);
    assert_eq!(client.get(format!("{url}/v1/health")).send().await.unwrap().status(), StatusCode::OK);
    for path in ["/v1/auth/challenge", "/v1/auth/verify"] {
        assert_eq!(client.post(format!("{url}{path}")).send().await.unwrap().status(), StatusCode::OK);
    }
    assert_eq!(relay.writes.load(Ordering::Relaxed), 10, "reads or synthetic authentication changed mutation count");
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

fn bot(id: &str) -> Bot {
    Bot {
        id: id.into(), name: id.into(), description: String::new(), symbol_name: String::new(),
        accent: String::new(), avatar: None, look: None, runner_id: "runner".into(),
        provider: "deepseek".into(), model: None, thinking: None, legacy_instructions: String::new(),
        workdir: None, capabilities: Capabilities::default(), created_at: 1.0,
    }
}

async fn checkpoint(lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>) -> Value {
    timeout(Duration::from_secs(10), async {
        loop {
            let line = lines.next_line().await.unwrap().expect("child exited before checkpoint");
            if let Some((_, value)) = line.split_once(CHECKPOINT) {
                return serde_json::from_str(value).unwrap();
            }
        }
    }).await.expect("child checkpoint deadline")
}

#[tokio::test]
async fn queued_encrypted_intent_and_conflict_survive_process_stop_and_reload() {
    for _ in 0..3 {
        let home = tempfile::tempdir().unwrap();
        let app = App::load(Config { home: home.path().into(), port: 0 }).unwrap();
        beans::identity::create(&app, Some("Runner".into())).unwrap();
        let initial = app.store.queued_roster().unwrap().unwrap();
        app.store.remove_outbox_roster_with_state(&initial.id, &app.state.lock().unwrap().clone(), &[]).unwrap();
        let base = RosterBlob {
            bots: vec![bot("bot0"), bot("bot1")],
            chats: vec![ChatMeta { id: "group".into(), kind: "group".into(), title: None,
                bot_ids: vec!["bot0".into(), "bot1".into()], owner_bot_id: Some("bot0".into()),
                description: None, is_pinned: false, created_at: 1.0 }],
            ..Default::default()
        };
        {
            let mut state = app.state.lock().unwrap();
            state.bots = base.bots.clone();
            state.chats = vec![Chat { meta: base.chats[0].clone(), unread_count: 0, usage: None, compactions: vec![] }];
            state.roster_slot_seq = 7;
        }
        app.save_state_now();
        app.store.observe_roster(7, &base).unwrap();
        // Reuse c731's canonical disjoint-removal case through the actual membership API.
        beans::api::dispatch(&app, "chats.remove_bot", json!({"chat_id": "group", "bot_id": "bot0"})).await.unwrap();
        let mut local = base.chats[0].clone();
        local.bot_ids = vec!["bot1".into()];
        local.owner_bot_id = Some("bot1".into());
        assert_eq!(app.chat("group").unwrap().meta, local);
        let dek = app.dek().unwrap();
        let queued = app.store.queued_roster().unwrap().unwrap();
        let intent: RosterBlob = beans::crypto::decrypt_json(&dek, "roster", &queued.ciphertext).unwrap();
        assert_eq!(intent.chats, vec![local]);
        assert_eq!(intent.bots, base.bots);
        app.push_file_blob("group-file".into(), Some("group"),
            beans::crypto::encrypt(&dek, "file", b"offline attachment").unwrap());
        let mut remote = base.clone();
        remote.chats[0].bot_ids = vec!["bot0".into()];
        let remote_blob = json!({"id": "remote-roster", "kind": "roster", "recipient_machine_pubkey": null,
            "ciphertext": beans::keys::b64(&beans::crypto::encrypt_json(&dek, "roster", &remote).unwrap()),
            "seq": 8, "created_at": 1});
        let relay = Relay { roster: remote_blob.clone(), upgrades: Arc::new(AtomicUsize::new(0)), writes: Arc::new(AtomicUsize::new(0)) };
        let router = relay_router(relay.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        app.machine.lock().unwrap().as_mut().unwrap().registered = true;
        app.save_machine().unwrap();
        app.set_relay_url(Some(url.clone())).unwrap();
        app.save_state_now();
        let expected = persisted(&app);
        assert_eq!(expected["baseline"], serde_json::to_value(Some((7, base))).unwrap());
        drop(app);
        for _ in 0..2 {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command.env_clear().env("HOME", home.path()).env("USERPROFILE", home.path())
                .env("BEANS_RECONNECT_FIXTURE_HOME", home.path()).env("RUST_LOG", "off");
            if let Some(root) = std::env::var_os("SystemRoot") { command.env("SystemRoot", root); }
            let mut child = command.args(["--exact", "reconnect_process_worker", "--ignored", "--nocapture"])
                .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::inherit()).kill_on_drop(true).spawn().unwrap();
            let pid = child.id().unwrap();
            let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
            let loaded = checkpoint(&mut lines).await;
            assert_eq!(loaded["pid"], pid);
            assert_eq!(loaded["persisted"], expected);
            for _ in 0..2 {
                let failed = checkpoint(&mut lines).await;
                assert_eq!(failed["pid"], pid);
                assert_eq!(failed["persisted"], expected);
                assert_eq!(failed["error"], json!({"message": CONFLICT, "unknown_machine": false}));
            }
            child.kill().await.unwrap();
            assert!(!child.wait().await.unwrap().success());
            // Only after actual process death: reopen the persisted store and compare all bytes.
            let reopened = App::load(Config { home: home.path().into(), port: 0 }).unwrap();
            assert_eq!(persisted(&reopened), expected);
            let (blobs, seq) = reopened.relay.list_blobs(&url, "synthetic-token", 0, "roster").await.unwrap();
            assert_eq!(seq, 8);
            let observed: Vec<_> = blobs.into_iter().map(|blob| json!({"id": blob.id, "kind": blob.kind,
                "recipient_machine_pubkey": blob.recipient_machine_pubkey, "ciphertext": blob.ciphertext,
                "seq": blob.seq, "created_at": blob.created_at})).collect();
            assert_eq!(observed, vec![remote_blob.clone()]);
            drop(reopened);
        }
        assert!(relay.upgrades.load(Ordering::Relaxed) >= 4);
        assert_eq!(relay.writes.load(Ordering::Relaxed), 0);
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }
}
