use super::*;
use crate::config::Config;
use axum::{http::StatusCode, routing::{delete, get, post}, Json, Router};
use serde_json::json;
use std::sync::{Mutex, atomic::{AtomicBool, AtomicUsize}};
use tokio::sync::oneshot;

fn fixture(url: &str) -> (tempfile::TempDir, Arc<App>, RelayAuthority) {
    let home = tempfile::tempdir().unwrap();
    let app = App::load(Config { home: home.path().into(), port: 0 }).unwrap();
    app.settings.lock().unwrap().relay_url = Some(url.into());
    crate::identity::create(&app, Some("Synthetic owner".into())).unwrap();
    add_peer(&app, "old");
    let owner = RelayAuthority::capture(&app, url, &app.this_device_id().unwrap()).unwrap();
    (home, app, owner)
}

fn add_peer(app: &Arc<App>, job: &str) {
    {
        let mut state = app.state.lock().unwrap();
        state.devices.push(Device { id: "revoked".into(), name: "Synthetic peer".into(), ..Default::default() });
        state.listed_machines.insert("revoked".into(), 1);
        state.turns_online.insert("revoked".into());
        app.store.save_state(&state).unwrap();
    }
    app.set_device_turns("revoked", vec![LiveTurn { job_id: job.into(), chat_id: "chat".into(), bot_id: "bot".into(), routine_id: None, activity: None }]);
}

fn replace_account(app: &Arc<App>) {
    app.forget_identity().unwrap();
    crate::identity::create(app, Some("Synthetic replacement".into())).unwrap();
    add_peer(app, "replacement");
}

fn assert_peer(app: &Arc<App>, job: &str) {
    let state = app.state.lock().unwrap();
    assert!(state.devices.iter().any(|d| d.id == "revoked"));
    assert_eq!(state.device_turns["revoked"][0].job_id, job);
    drop(state);
    assert_eq!(app.store.load_state().unwrap().device_turns["revoked"][0].job_id, job);
}

#[test]
fn unpair_ownership_rejects_replacement_and_relay_away_back() {
    for transition in ["replacement", "relay"] {
        let (_home, app, owner) = fixture("http://127.0.0.1:19001");
        if transition == "replacement" { replace_account(&app); }
        else {
            app.set_relay_url(Some("http://127.0.0.1:19002".into())).unwrap();
            app.set_relay_url(Some(owner.url.clone())).unwrap();
        }
        assert!(commit_unpair(&app, "revoked", &owner).is_err());
        assert_peer(&app, if transition == "replacement" { "replacement" } else { "old" });
    }
}

#[test]
fn unpair_ownership_turn_purge_rolls_back_durably_and_can_retry() {
    let (_home, app, owner) = fixture("http://127.0.0.1:19001");
    let db = rusqlite::Connection::open(app.config.database_path()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_unpair BEFORE DELETE ON device_turns BEGIN SELECT RAISE(ABORT, 'synthetic turn failure'); END;").unwrap();
    let error = commit_unpair(&app, "revoked", &owner).unwrap().unwrap_err();
    assert!(error.message.contains("synthetic turn failure"));
    assert!(!app.state.lock().unwrap().devices.iter().any(|d| d.id == "revoked"));
    assert!(!app.state.lock().unwrap().device_turns.contains_key("revoked"));
    let saved = app.store.load_state().unwrap();
    assert!(saved.devices.iter().any(|d| d.id == "revoked"));
    assert_eq!(saved.device_turns["revoked"][0].job_id, "old");
    db.execute_batch("DROP TRIGGER fail_unpair").unwrap();
    commit_unpair(&app, "revoked", &owner).unwrap().unwrap();
    let saved = app.store.load_state().unwrap();
    assert!(!saved.devices.iter().any(|d| d.id == "revoked"));
    assert!(!saved.device_turns.contains_key("revoked"));
}

#[test]
fn unpair_ownership_success_survives_reopen_and_tail_cannot_purge_new_turns() {
    let (home, app, owner) = fixture("http://127.0.0.1:19001");
    commit_unpair(&app, "revoked", &owner).unwrap().unwrap();
    drop(app);
    let app = App::load(Config { home: home.path().into(), port: 0 }).unwrap();
    assert!(!app.state.lock().unwrap().devices.iter().any(|d| d.id == "revoked"));
    assert!(!app.state.lock().unwrap().device_turns.contains_key("revoked"));
    replace_account(&app);
    let before = std::fs::read(app.config.machine_path()).unwrap();
    publish_unpair(&app);
    assert_peer(&app, "replacement");
    assert_eq!(std::fs::read(app.config.machine_path()).unwrap(), before);
}

#[test]
fn relay_ownership_failed_setting_write_retains_source_and_generation() {
    let (_home, app, owner) = fixture("http://127.0.0.1:19001");
    std::fs::create_dir(app.config.settings_path()).unwrap();
    assert!(app.set_relay_url(Some("http://127.0.0.1:19002".into())).is_err());
    assert_eq!(app.relay_url().as_deref(), Some(owner.url.as_str()));
    owner.with_current(&app, || Ok(())).unwrap();
}

struct HeldRelay {
    url: String,
    arrived: oneshot::Receiver<()>,
    release: oneshot::Sender<()>,
    registrations: Arc<AtomicUsize>,
    revocations: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
}

async fn held_relay(stage: &'static str) -> HeldRelay {
    let (arrived, waiting) = oneshot::channel();
    let (release, blocked) = oneshot::channel();
    let arrived = Arc::new(Mutex::new(Some(arrived)));
    let blocked = Arc::new(tokio::sync::Mutex::new(Some(blocked)));
    let pause = move || {
        let (arrived, blocked) = (arrived.clone(), blocked.clone());
        async move {
            if let Some(notify) = arrived.lock().unwrap().take() { let _ = notify.send(()); }
            if let Some(wait) = blocked.lock().await.take() { let _ = wait.await; }
        }
    };
    let registrations = Arc::new(AtomicUsize::new(0));
    let revocations = Arc::new(AtomicUsize::new(0));
    let registered = Arc::new(AtomicBool::new(stage == "revoke"));
    let registration = {
        let (pause, count, registered) = (pause.clone(), registrations.clone(), registered.clone());
        move || { let (pause, count, registered) = (pause.clone(), count.clone(), registered.clone()); async move {
            count.fetch_add(1, Ordering::Relaxed);
            if stage == "register" { pause().await; }
            registered.store(true, Ordering::Relaxed);
            Json(json!({}))
        }}
    };
    let challenge = {
        let (pause, registered) = (pause.clone(), registered.clone());
        move || { let (pause, registered) = (pause.clone(), registered.clone()); async move {
            if stage == "challenge" { pause().await; }
            if registered.load(Ordering::Relaxed) { (StatusCode::OK, Json(json!({"nonce":"synthetic"}))) }
            else { (StatusCode::NOT_FOUND, Json(json!({"error":"unknown synthetic machine"}))) }
        }}
    };
    let revoke = {
        let count = revocations.clone();
        move || { let (pause, count) = (pause.clone(), count.clone()); async move {
            count.fetch_add(1, Ordering::Relaxed);
            if stage == "revoke" { pause().await; }
            Json(json!({}))
        }}
    };
    let router = Router::new()
        .route("/v1/health", get(|| async { Json(json!({"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":1})) }))
        .route("/v1/identities", post(registration))
        .route("/v1/auth/challenge", post(challenge))
        .route("/v1/auth/verify", post(|| async { Json(json!({"token":"synthetic-owner-token"})) }))
        .route("/v1/machines/{id}", delete(revoke));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    HeldRelay { url, arrived: waiting, release, registrations, revocations, server }
}

#[tokio::test]
async fn unpair_ownership_network_awaits_cannot_publish_into_replacement_source() {
    for stage in ["challenge", "register", "revoke"] {
        for transition in ["none", "replacement", "relay"] {
            let relay = held_relay(stage).await;
            let (_home, app, _) = fixture(&relay.url);
            let held = app.clone();
            let operation = tokio::spawn(async move { unpair_device(&held, "revoked").await });
            tokio::time::timeout(std::time::Duration::from_secs(5), relay.arrived).await.unwrap().unwrap();
            match transition {
                "replacement" => replace_account(&app),
                "relay" => {
                    app.set_relay_url(Some("http://127.0.0.1:19002".into())).unwrap();
                    app.set_relay_url(Some(relay.url.clone())).unwrap();
                }
                _ => {}
            }
            let before = std::fs::read(app.config.machine_path()).unwrap();
            let outbox_before = app.store.first_outbox().unwrap().map(|x| x.id);
            relay.release.send(()).unwrap();
            let result = tokio::time::timeout(std::time::Duration::from_secs(5), operation).await.unwrap().unwrap();
            if transition == "none" {
                result.unwrap();
                assert!(!app.state.lock().unwrap().devices.iter().any(|d| d.id == "revoked"));
                assert!(!app.store.load_state().unwrap().device_turns.contains_key("revoked"));
                assert_eq!(relay.revocations.load(Ordering::Relaxed), 1);
            } else {
                assert!(result.is_err(), "{stage}/{transition}");
                assert_peer(&app, if transition == "replacement" { "replacement" } else { "old" });
                assert_eq!(std::fs::read(app.config.machine_path()).unwrap(), before);
                assert_eq!(app.store.first_outbox().unwrap().map(|x| x.id), outbox_before);
                assert_eq!(relay.revocations.load(Ordering::Relaxed), usize::from(stage == "revoke"));
                if stage == "challenge" { assert_eq!(relay.registrations.load(Ordering::Relaxed), 0); }
            }
            relay.server.abort();
        }
    }
}

#[test]
fn unpair_alias_intent_and_turn_removal_commit_together() {
    let (home, app, owner) = fixture("http://127.0.0.1:19001");
    app.set_device_name("revoked", "Office").unwrap();
    let queued = app.store.queued_roster().unwrap().unwrap();
    let db = rusqlite::Connection::open(app.config.database_path()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_alias_unpair BEFORE UPDATE ON outbox BEGIN SELECT RAISE(ABORT, 'synthetic alias unpair failure'); END;").unwrap();
    assert!(commit_unpair(&app, "revoked", &owner).unwrap().is_err());
    assert!(!app.state.lock().unwrap().devices.iter().any(|d| d.id == "revoked"));
    let saved = app.store.load_state().unwrap();
    assert_eq!(saved.device_names["revoked"], "Office");
    assert!(saved.device_turns.contains_key("revoked"));
    assert_eq!(app.store.queued_roster().unwrap().unwrap().id, queued.id);
    db.execute_batch("DROP TRIGGER fail_alias_unpair").unwrap();
    commit_unpair(&app, "revoked", &owner).unwrap().unwrap();
    drop(db);
    drop(app);
    let reopened = App::load(Config { home: home.path().into(), port: 0 }).unwrap();
    let saved = reopened.store.load_state().unwrap();
    assert!(!saved.device_names.contains_key("revoked"));
    assert!(!saved.device_turns.contains_key("revoked"));
    let item = reopened.store.queued_roster().unwrap().unwrap();
    assert_ne!(item.id, queued.id);
    assert!(saved.applied_blob_ids.contains(&item.id));
    let roster: RosterBlob = crate::crypto::decrypt_json(&reopened.dek().unwrap(), "roster", &item.ciphertext).unwrap();
    assert!(!roster.device_names.contains_key("revoked"));
}
