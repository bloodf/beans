//! No-provider baseline: local reload and direct encrypted reconciliation, not relay pairing.
#![cfg(feature = "runner")]

use beans::{
    api, app::App, config::Config, identity, keys,
    model::{AutoReview, AutoReviewRule, Body, Capabilities},
    plugins::mcp::{self, Decision}, relay::BlobIn, sync,
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("beans-policy-history-{}", uuid::Uuid::new_v4())))
    }
    fn load(&self) -> Arc<App> {
        App::load(Config { home: self.0.clone(), port: 0 }).unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// Read the production encrypted queue without consuming it or copying its engine.
// LocalStore::outbox is cfg(test), unavailable to integration-test consumers.
fn queued(app: &App) -> Vec<BlobIn> {
    let db = rusqlite::Connection::open(app.config.home.join("beans.sqlite3")).unwrap();
    let mut statement = db.prepare(
        "SELECT id, kind, ciphertext FROM outbox WHERE kind IN ('roster', 'chat', 'policy') ORDER BY position",
    ).unwrap();
    statement.query_map([], |row| {
        let ciphertext: Vec<u8> = row.get(2)?;
        Ok(BlobIn {
            id: row.get(0)?, kind: row.get(1)?, ciphertext: keys::b64(&ciphertext),
            recipient_machine_pubkey: None, seq: 0, created_at: 0,
        })
    }).unwrap().collect::<Result<_, _>>().unwrap()
}

fn reconcile(receiver: &Arc<App>, blobs: &[BlobIn], seq: &mut i64) {
    let machine = receiver.machine_file().unwrap();
    for blob in blobs {
        *seq += 1;
        let mut blob = blob.clone();
        blob.seq = *seq;
        sync::apply_blob(receiver, &machine, &blob);
    }
}

async fn answer(app: &Arc<App>, chat: &str, bot: &str, decision: Decision) -> String {
    // Watching suppresses push work before any relay access.
    app.set_watched_chat(Some(chat.into()));
    let mut events = app.events.subscribe();
    let rule = (decision == Decision::Always).then(|| AutoReviewRule {
        id: "baseline-rule".into(), text: "Allow synthetic fixture action".into(),
        behavior: "allow".into(), tool: Some("baseline/action".into()),
    });
    let waiter_app = app.clone();
    let chat_id = chat.to_string();
    let bot_id = bot.to_string();
    let waiter = tokio::spawn(async move {
        mcp::ask_with_rule(
            &waiter_app, &chat_id, &bot_id, "baseline", "Baseline", "action",
            "Synthetic action; no external effect", json!({}), None, rule,
            &CancellationToken::new(),
        ).await
    });
    let message_id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            // Subscribe before spawning; inspect persisted cards after each production event.
            events.recv().await.unwrap();
            if let Some(message) = app.store.all(chat).unwrap().into_iter().find(|message| {
                matches!(&message.body, Body::Permission { decision, .. } if decision == "pending")
            }) {
                break message.id;
            }
        }
    }).await.expect("permission card never appeared");
    let wire = match decision {
        Decision::Allowed => "allow", Decision::Denied => "deny", Decision::Always => "always",
        _ => unreachable!(),
    };
    api::dispatch(app, "chats.permission", json!({
        "chat_id": chat, "message_id": message_id, "decision": wire,
    })).await.unwrap();
    assert_eq!(tokio::time::timeout(Duration::from_secs(5), waiter).await.unwrap().unwrap(), decision);
    message_id
}

fn assert_cards(app: &App, chat: &str, cards: &[(String, Decision)]) {
    for (id, expected) in cards {
        let message = app.message(chat, id).expect("answered card survives reload/reconciliation");
        let Body::Permission { decision, .. } = &message.body else { panic!("not a permission card") };
        assert_eq!(decision, expected.as_str());
        let Body::Permission { decision, arguments, .. } = message.for_app().body else { unreachable!() };
        assert_eq!(decision, expected.as_str());
        assert!(arguments.is_null(), "client projection omits reviewed arguments");
    }
    let rule = app.state.lock().unwrap().auto_review.rules[0].clone();
    assert_eq!(rule, AutoReviewRule {
        id: "baseline-rule".into(), text: "Allow synthetic fixture action".into(),
        behavior: "allow".into(), tool: Some("baseline/action".into()),
    });
}

#[tokio::test]
async fn policy_history_survives_reload_and_direct_encrypted_reconciliation() {
    let home_a = Scratch::new();
    let home_b = Scratch::new();
    let mut a = home_a.load();
    identity::create(&a, Some("Synthetic A".into())).unwrap();
    a.set_auto_review(AutoReview { is_enabled: false, rules: vec![] });
    let bot = a.state.lock().unwrap().bots[0].id.clone();
    let dm = a.state.lock().unwrap().chats[0].meta.id.clone();
    let runner = a.this_device_id().unwrap();
    api::dispatch(&a, "bots.create", json!({"id":"survivor", "name":"Survivor", "runner_id":runner})).await.unwrap();
    api::dispatch(&a, "chats.create", json!({"id":"baseline-group", "bot_ids":[bot,"survivor"]})).await.unwrap();

    let mut b = home_b.load();
    // Synthetic account-key provisioning, deliberately not pair.accept or relay transport.
    let mut machine = a.machine_file().unwrap();
    machine.machine_secret = keys::b64(&keys::Machine::generate().secret);
    machine.name = "Synthetic B".into();
    *b.machine.lock().unwrap() = Some(machine);
    b.save_machine().unwrap();
    assert_ne!(a.this_device_id(), b.this_device_id());
    let mut seq = 0;
    reconcile(&b, &queued(&a), &mut seq);

    let mut cards = Vec::new();
    for decision in [Decision::Allowed, Decision::Denied, Decision::Always] {
        cards.push((answer(&a, "baseline-group", &bot, decision).await, decision));
    }
    let dm_card = answer(&a, &dm, &bot, Decision::Denied).await;
    let stale = queued(&a).into_iter().find(|blob| blob.kind == "roster").unwrap();
    reconcile(&b, &queued(&a), &mut seq);
    drop(a);
    drop(b);
    a = home_a.load();
    b = home_b.load();
    for app in [&a, &b] {
        assert_cards(app, "baseline-group", &cards);
        assert!(app.message(&dm, &dm_card).is_some());
    }

    let restricted = Capabilities { shell: false, write: false, plugins: Some(vec![]) };
    a.set_paused(true);
    a.update_bot(&bot, |bot| bot.capabilities = restricted.clone()).unwrap();
    reconcile(&b, &queued(&a), &mut seq);
    drop(a);
    drop(b);
    a = home_a.load();
    b = home_b.load();
    for app in [&a, &b] {
        assert!(app.is_paused());
        assert_eq!(app.bot(&bot).unwrap().capabilities, restricted);
    }

    // Explicit newer Resume/relax from B propagates back; old restrictions cannot win.
    b.set_paused(false);
    b.update_bot(&bot, |bot| bot.capabilities = Capabilities::default()).unwrap();
    reconcile(&a, &queued(&b), &mut seq);
    drop(a);
    drop(b);
    a = home_a.load();
    b = home_b.load();
    for app in [&a, &b] {
        assert!(!app.is_paused());
        assert_eq!(app.bot(&bot).unwrap().capabilities, Capabilities::default());
    }
    a.delete_bot(&bot).unwrap();
    reconcile(&b, &queued(&a), &mut seq);
    let mut stale = stale;
    stale.id = uuid::Uuid::new_v4().to_string();
    reconcile(&b, &[stale], &mut seq);
    drop(a);
    drop(b);
    a = home_a.load();
    b = home_b.load();
    for app in [&a, &b] {
        assert!(!app.is_paused(), "newer explicit Resume survives reload");
        assert!(app.bot(&bot).is_none(), "stale roster cannot resurrect deleted bot");
        assert!(app.state.lock().unwrap().deleted_bot_versions.contains_key(&bot));
        assert!(app.chat(&dm).is_none());
        assert!(app.store.all(&dm).unwrap().is_empty(), "deleted DM loses its history");
        assert_eq!(app.chat("baseline-group").unwrap().meta.bot_ids, vec!["survivor"]);
        assert_eq!(app.chat("baseline-group").unwrap().meta.owner_bot_id.as_deref(), Some("survivor"));
        assert_cards(app, "baseline-group", &cards);
    }
    // Auto-review model verdict history is absent, not a positive persistence result.
    // This fixture never calls review::review or a provider. It proves only human outcomes,
    // current rule/policy projections and deletion retention; see evidence/qa-policy.md.
}
