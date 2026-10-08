//! Create or restore the identity on this Device.

use std::sync::Arc;

use crate::app::{upsert_device, App};
use crate::config::now_unix;
use crate::events::Event;
use crate::keys::{self, Identity, IdentityFile, Machine, MachineFile};
use crate::model::host_facts;

fn machine_file_for(identity_pubkey: &str, content_pubkey: &str, machine: &Machine, dek: &[u8; 32], registered: bool, relay_url: Option<String>, name: Option<String>) -> MachineFile {
    let (host_name, os, os_version, model) = host_facts();
    MachineFile { format: crate::config::Format::BeansV2, machine_secret: keys::b64(&machine.secret),
    identity_pubkey: identity_pubkey.to_string(),
    content_pubkey: content_pubkey.to_string(),
    account_dek: keys::b64(dek),
    name: name.filter(|n| !n.trim().is_empty()).unwrap_or(host_name),
    os,
    os_version,
    model,
    registered,
    relay_url,
    created_at: now_unix(), }
}

/// New identity, new machine, new account key. Returns the backup phrase.
pub fn create(app: &Arc<App>, device_name: Option<String>) -> anyhow::Result<Vec<String>> {
    let _admission = app.plugin_admission(None).map_err(anyhow::Error::msg)?;
    if app.has_identity() {
        anyhow::bail!("This Device already has an identity. Remove {} first.", app.config.home.display());
    }
    let identity = Identity::generate();
    let machine = Machine::generate();
    let dek = keys::generate_dek();

    let machine_file = machine_file_for(&identity.pubkey(), &identity.content_pubkey(), &machine, &dek, false, app.relay_url(), device_name);
    *app.identity.lock().unwrap() = Some(IdentityFile::new(&identity));
    *app.machine.lock().unwrap() = Some(machine_file);
    app.save_identity()?;
    app.save_machine()?;

    // Fresh store; the sequence starts at zero for a new identity.
    {
        let mut state = app.state.lock().unwrap();
        *state = Default::default();
    }
    app.store.clear()?;
    if let Some(device) = app.local_device() {
        upsert_device(&mut app.state.lock().unwrap().devices, device);
    }
    app.save_state();

    // The DEK wrapped to the content key, so a restore can unwrap it. Uploaded when a relay is
    // reachable; harmless to keep queued until then.
    let sealed = crate::crypto::seal_json(&identity.content_pubkey(), &keys::KeyRecord {
        format: crate::config::Format::BeansV2, account_dek: keys::b64(&dek),
    })?;
    app.push_blob("key", None, sealed);
    app.push_machine_blob_if_changed();
    create_lead_bot(app);

    app.emit(Event::IdentityChanged { has_identity: true });
    app.emit(Event::Snapshot(app.snapshot()));
    Ok(identity.phrase())
}

pub const LEAD_BOT_NAME: &str = "Chef";

/// A new account starts with one general-purpose bot to talk to. It is an ordinary bot with a
/// default profile on this Runner.
fn create_lead_bot(app: &Arc<App>) {
    let Some(runner_id) = app.this_device_id() else { return };
    let bot = crate::model::Bot {
        id: String::new(),
        name: LEAD_BOT_NAME.into(),
        description: "Chief of staff. Plans the work and delegates each task to the right teammate, proposing a new one when none fits. Does hands-on work when necessary.".into(),
        symbol_name: "sparkles".into(),
        accent: "indigo".into(),
        avatar: None,
        look: None,
        runner_id,
        provider: "deepseek".into(),
        model: None,
        thinking: None,
        legacy_instructions: String::new(),
        workdir: None,
        capabilities: Default::default(),
        created_at: 0.0,
    };
    match app.create_bot_with_dm(bot, None) {
        Ok((_, chat)) => {
            let _ = app.update_chat_meta(&chat.meta.id, |meta| meta.is_pinned = true);
        }
        Err(error) => tracing::warn!(%error, "creating the lead bot"),
    }
}

/// Restore from the backup phrase: re-derive keys, attest this machine, unwrap the DEK.
pub async fn restore(app: &Arc<App>, phrase: &str, device_name: Option<String>) -> anyhow::Result<()> {
    let incarnation = {
        let admission = app.plugin_admission(None).map_err(anyhow::Error::msg)?;
        if app.has_identity() { anyhow::bail!("This Device already has an identity."); }
        admission.get().0
    };
    let identity = Identity::from_master(keys::secret_from_phrase(phrase)?);
    let url = app.relay_url().ok_or_else(|| anyhow::anyhow!("Set a relay URL first. Restoring unwraps the account key from the relay."))?;
    let machine = Machine::generate();

    app.relay.register(&url, &identity, &machine.pubkey(), &machine.box_pubkey()).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    let dek = crate::sync::fetch_dek(app, &url, &identity, &machine).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    let _admission = app.plugin_admission(Some(incarnation)).map_err(anyhow::Error::msg)?;
    if app.has_identity() { anyhow::bail!("This Device already has an identity."); }

    let machine_file = machine_file_for(&identity.pubkey(), &identity.content_pubkey(), &machine, &dek, true, Some(url), device_name);
    *app.identity.lock().unwrap() = Some(IdentityFile::new(&identity));
    *app.machine.lock().unwrap() = Some(machine_file);
    app.save_identity()?;
    app.save_machine()?;
    {
        let mut state = app.state.lock().unwrap();
        *state = Default::default();
    }
    app.store.clear()?;
    if let Some(device) = app.local_device() {
        upsert_device(&mut app.state.lock().unwrap().devices, device);
    }
    app.save_state();
    app.push_machine_blob_if_changed();
    app.outbox_notify.notify_waiters();

    app.emit(Event::IdentityChanged { has_identity: true });
    app.emit(Event::Snapshot(app.snapshot()));
    Ok(())
}

#[cfg(test)]
mod publication_tests {
    use super::*;

    #[tokio::test]
    async fn restore_publication_rejects_forget_and_competing_create() {
        use axum::{routing::{get, post}, Json, Router};
        for transition in ["none", "forget", "create"] {
            let home = std::env::temp_dir().join(format!("beans-restore-fence-{}", uuid::Uuid::new_v4()));
            let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
            let identity = Identity::generate();
            let sealed = crate::crypto::seal_json(&identity.content_pubkey(), &keys::KeyRecord {
                format: crate::config::Format::BeansV2, account_dek: keys::b64(&[42; 32]),
            }).unwrap();
            let (arrived, waiting) = tokio::sync::oneshot::channel();
            let (release, blocked) = tokio::sync::oneshot::channel();
            let arrived = Arc::new(std::sync::Mutex::new(Some(arrived)));
            let blocked = Arc::new(tokio::sync::Mutex::new(Some(blocked)));
            let router = Router::new()
                .route("/v1/health", get(|| async { Json(serde_json::json!({"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":1})) }))
                .route("/v1/identities", post(|| async { Json(serde_json::json!({})) }))
                .route("/v1/auth/challenge", post(|| async { Json(serde_json::json!({"nonce":"synthetic"})) }))
                .route("/v1/auth/verify", post(|| async { Json(serde_json::json!({"token":"synthetic"})) }))
                .route("/v1/blobs", get(move || {
                    let (arrived, blocked, sealed) = (arrived.clone(), blocked.clone(), sealed.clone());
                    async move {
                        arrived.lock().unwrap().take().unwrap().send(()).unwrap();
                        blocked.lock().await.take().unwrap().await.unwrap();
                        Json(serde_json::json!({"seq":1,"blobs":[{"id":"synthetic-key","kind":"key","seq":1,"ciphertext":keys::b64(&sealed),"created_at":0}]}))
                    }
                }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            app.set_relay_url(Some(format!("http://{}", listener.local_addr().unwrap()))).unwrap();
            let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let (held, phrase) = (app.clone(), identity.phrase().join(" "));
            let restoring = tokio::spawn(async move { restore(&held, &phrase, Some("Synthetic restore".into())).await });
            waiting.await.unwrap();
            let competing = match transition {
                "forget" => { app.forget_identity().unwrap(); None },
                "create" => { create(&app, Some("Synthetic replacement".into())).unwrap(); app.this_device_id() },
                _ => None,
            };
            release.send(()).unwrap();
            let result = restoring.await.unwrap();
            match transition {
                "none" => { result.unwrap(); assert_eq!(app.machine_file().unwrap().identity_pubkey, identity.pubkey()); },
                "forget" => { assert!(result.is_err()); assert!(app.machine_file().is_none()); assert!(!app.config.identity_path().exists()); },
                _ => { assert!(result.is_err()); assert_eq!(app.this_device_id(), competing); },
            }
            server.abort();
            drop(app);
            std::fs::remove_dir_all(home).unwrap();
        }
    }

    #[test]
    fn create_waits_for_reset_publication_boundary() {
        let home = std::env::temp_dir().join(format!("beans-create-fence-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        create(&app, Some("Synthetic A".into())).unwrap();
        let guard = app.plugin_lifecycle.lock();
        let (started, waiting) = std::sync::mpsc::channel();
        let held = app.clone();
        let creating = std::thread::spawn(move || { started.send(()).unwrap(); create(&held, Some("Synthetic B".into())) });
        waiting.recv().unwrap();
        app.forget_identity().unwrap();
        drop(guard);
        creating.join().unwrap().unwrap();
        assert_eq!(app.machine_file().unwrap().name, "Synthetic B");
        assert!(app.plugins.lock().unwrap().installed().is_empty());
        drop(app);
        std::fs::remove_dir_all(home).unwrap();
    }
}
