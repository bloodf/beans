//! Pairing. A paired Device (A) publishes a pairing string; the joining Device (B) posts a
//! sealed request to the relay's pairing mailbox; A attests B's machine and seals the account
//! key back to it. A attests with the identity key when it holds it, and with its own machine
//! key otherwise.

use std::sync::Arc;

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::app::{upsert_device, App, PairingStatus, PendingPairing};
use crate::config::now_unix;
use crate::events::Event;
use crate::keys::{self, Machine, MachineFile};
use crate::model::{host_facts, Device, PairReply, PairRequest};

const PAIR_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);
const POLL: std::time::Duration = std::time::Duration::from_millis(1500);

pub fn parse_pairing_string(text: &str) -> anyhow::Result<(String, String, String, String)> {
    let query = text.trim().strip_prefix("beans://pair?")
        .ok_or_else(|| anyhow::anyhow!("That is not a Beans v2 pairing string"))?;
    let mut fields = std::collections::BTreeMap::new();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').ok_or_else(|| anyhow::anyhow!("Invalid pairing field"))?;
        if !matches!(key, "v" | "relay" | "id" | "ek" | "n") || fields.insert(key, percent_decode(value)?).is_some() {
            anyhow::bail!("Unknown or duplicate pairing field");
        }
    }
    if fields.get("v").map(String::as_str) != Some("2") {
        anyhow::bail!("Beans v2 pairing code required; incompatible pairing format");
    }
    let mut required = |key| fields.remove(key).filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Pairing string is missing {key}"));
    let relay = required("relay")?;
    let id = required("id")?;
    let ek = required("ek")?;
    let nonce = required("n")?;
    let url = reqwest::Url::parse(&relay)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none()
        || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        anyhow::bail!("Invalid pairing relay URL");
    }
    if id.len() != 43 || ek.len() != 43 {
        anyhow::bail!("Pairing keys must be canonical base64url");
    }
    keys::unb64_32(&id)?;
    keys::unb64_32(&ek)?;
    if nonce.len() > 256 || !nonce.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_') {
        anyhow::bail!("Invalid pairing nonce");
    }
    Ok((relay, id, ek, nonce))
}

fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn percent_decode(value: &str) -> anyhow::Result<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let encoded = bytes.get(i + 1..i + 3).ok_or_else(|| anyhow::anyhow!("Invalid pairing escape"))?;
            let encoded = std::str::from_utf8(encoded)?;
            out.push(u8::from_str_radix(encoded, 16)?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Ok(String::from_utf8(out)?)
}

/// A: create the mailbox and start waiting. Returns the nonce and the pairing string. Any
/// paired Device can; one without the identity key needs a relay that takes its attestation.
pub async fn start(app: Arc<App>) -> anyhow::Result<(String, String)> {
    let url = app.relay_url().ok_or_else(|| anyhow::anyhow!("Set a relay URL first. Pairing runs through the relay."))?;
    let machine_file = app.machine_file().ok_or_else(|| anyhow::anyhow!("No identity on this Device"))?;
    let machine = machine_file.machine()?;
    app.relay.health(&url).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    if !machine_file.registered {
        crate::sync::ensure_registered(&app, &url).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    let token = crate::sync::token_or_register(&app, &url, &machine).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    let nonce = app.relay.pair_create(&url, &token).await.map_err(|e| anyhow::anyhow!("{e}"))?;

    let ephemeral = crypto_box::SecretKey::generate(&mut rand::rngs::OsRng);
    let ek = keys::b64(ephemeral.public_key().as_bytes());
    let pairing_string = format!(
        "beans://pair?v=2&relay={}&id={}&ek={}&n={}",
        percent_encode(&url),
        percent_encode(&machine_file.identity_pubkey),
        percent_encode(&ek),
        percent_encode(&nonce)
    );

    let cancel = CancellationToken::new();
    app.pairings.lock().unwrap().insert(nonce.clone(), PendingPairing { ephemeral, status: PairingStatus::Waiting, cancel: cancel.clone() });

    let waiter = app.clone();
    let waiter_nonce = nonce.clone();
    tokio::spawn(async move {
        let result = tokio::select! {
            _ = cancel.cancelled() => Err("Pairing cancelled".to_string()),
            result = tokio::time::timeout(PAIR_TIMEOUT, wait_for_request(&waiter, &url, &waiter_nonce)) => match result {
                Ok(result) => result,
                Err(_) => Err("Nobody joined within ten minutes".into()),
            },
        };
        let mut pairings = waiter.pairings.lock().unwrap();
        if let Some(pending) = pairings.get_mut(&waiter_nonce) {
            pending.status = match result {
                Ok(device) => PairingStatus::Completed { device },
                Err(error) => PairingStatus::Failed { error },
            };
        }
    });

    Ok((nonce, pairing_string))
}

async fn wait_for_request(app: &Arc<App>, url: &str, nonce: &str) -> Result<Value, String> {
    let identity = app.identity.lock().unwrap().clone().map(|file| file.identity()).transpose().map_err(|e| e.to_string())?;
    let machine_file = app.machine_file().ok_or("no machine")?;
    let machine = machine_file.machine().map_err(|e| e.to_string())?;
    loop {
        let token = crate::sync::token_or_register(app, url, &machine).await.map_err(|e| e.to_string())?;
        let request = match app.relay.pair_get_request(url, &token, nonce).await {
            Ok(request) => request,
            Err(error) if error.is_unauthorized() => {
                app.relay.forget_token();
                None
            }
            Err(error) => return Err(error.to_string()),
        };
        let Some(sealed) = request else {
            tokio::time::sleep(POLL).await;
            continue;
        };
        let ephemeral = {
            let pairings = app.pairings.lock().unwrap();
            let pending = pairings.get(nonce).ok_or("pairing vanished")?;
            crypto_box::SecretKey::from_bytes(pending.ephemeral.to_bytes())
        };
        let request: PairRequest = crate::crypto::unseal_json(&ephemeral, &sealed).map_err(|e| e.to_string())?;
        keys::verifying_key(&request.machine_pubkey).map_err(|_| "joining Device sent a bad key".to_string())?;
        if request.device.id != request.machine_pubkey {
            return Err("joining Device's metadata does not match its key".into());
        }

        // Attest B, then hand it the account key.
        match &identity {
            Some(identity) => app.relay.register(url, identity, &request.machine_pubkey, &request.box_pubkey).await,
            None => app.relay.attest(url, &token, &request.machine_pubkey, &request.box_pubkey).await,
        }
        .map_err(|e| e.to_string())?;
        let reply = PairReply { format: crate::config::Format::BeansV2, identity_pubkey: machine_file.identity_pubkey.clone(),
        content_pubkey: machine_file.content_pubkey.clone(),
        account_dek: machine_file.account_dek.clone(),
        relay_url: url.to_string(), };
        let sealed_reply = crate::crypto::seal_json(&request.box_pubkey, &reply).map_err(|e| e.to_string())?;
        app.relay.pair_post_reply(url, &token, nonce, &sealed_reply).await.map_err(|e| e.to_string())?;

        let mut device = request.device.clone();
        device.box_pubkey = request.box_pubkey.clone();
        device.updated_at = now_unix();
        {
            let mut state = app.state.lock().unwrap();
            upsert_device(&mut state.devices, device.clone());
            state.device_seen.insert(device.id.clone(), now_unix());
            // It opens its sync socket next; the relay's `machines` signal confirms it.
            state.device_online.insert(device.id.clone());
            // The new Device needs this machine's metadata, whatever the relay already holds.
            state.machine_blob_hash = None;
        }
        app.push_machine_blob_if_changed();
        app.roster_changed(true);
        let out = json!({ "id": device.id, "name": device.name, "os": device.os, "model": device.model });
        app.emit(Event::PairCompleted { nonce: nonce.to_string(), device: out.clone() });
        return Ok(out);
    }
}

pub fn status(app: &Arc<App>, nonce: &str) -> Value {
    let pairings = app.pairings.lock().unwrap();
    match pairings.get(nonce) {
        Some(pending) => serde_json::to_value(&pending.status).unwrap_or(Value::Null),
        None => json!({ "state": "failed", "error": "Unknown pairing" }),
    }
}

/// A stops waiting. The mailbox on the relay goes too, so a Device still polling it learns
/// the code is dead instead of waiting out the TTL.
pub fn cancel(app: &Arc<App>, nonce: &str) {
    let Some(pending) = app.pairings.lock().unwrap().remove(nonce) else { return };
    pending.cancel.cancel();
    let app = app.clone();
    let nonce = nonce.to_string();
    tokio::spawn(async move {
        let Some(url) = app.relay_url() else { return };
        let Some(machine) = app.machine_file().and_then(|file| file.machine().ok()) else { return };
        if let Ok(token) = crate::sync::token_or_register(&app, &url, &machine).await {
            let _ = app.relay.pair_delete(&url, &token, &nonce).await;
        }
    });
}

/// B: join an identity with a pairing string from A. Waits for A's reply; `abort` ends the
/// wait, and a newer `accept` replaces one still waiting.
pub async fn accept(app: Arc<App>, pairing_string: &str, device_name: Option<String>) -> anyhow::Result<Value> {
    if app.has_identity() {
        anyhow::bail!("This Device already belongs to an identity.");
    }
    let pairing = parse_pairing_string(pairing_string)?;
    let cancel = CancellationToken::new();
    if let Some(previous) = app.accepting.lock().unwrap().replace(cancel.clone()) {
        previous.cancel();
    }
    let result = tokio::select! {
        _ = cancel.cancelled() => Err(anyhow::anyhow!("Pairing cancelled")),
        result = join(&app, pairing, device_name) => result,
    };
    // Cancelled means a newer accept owns the slot, or abort already emptied it.
    if !cancel.is_cancelled() {
        app.accepting.lock().unwrap().take();
    }
    result
}

/// B gives up on the pairing it is waiting on.
pub fn abort(app: &Arc<App>) {
    if let Some(token) = app.accepting.lock().unwrap().take() {
        token.cancel();
    }
}

async fn join(app: &Arc<App>, pairing: (String, String, String, String), device_name: Option<String>) -> anyhow::Result<Value> {
    let incarnation = {
        let admission = app.plugin_admission(None).map_err(anyhow::Error::msg)?;
        if app.has_identity() { anyhow::bail!("This Device already has an identity."); }
        admission.get().0
    };
    let (relay_url, identity_pubkey, ek, nonce) = pairing;
    app.relay.health(&relay_url).await.map_err(|e| anyhow::anyhow!("{e}"))?;

    let machine = Machine::generate();
    let (host_name, os, os_version, model) = host_facts();
    let device = Device {
        id: machine.pubkey(),
        name: device_name.filter(|n| !n.trim().is_empty()).unwrap_or(host_name),
        model,
        os,
        os_version,
        box_pubkey: machine.box_pubkey(),
        plugins: Vec::new(),
        version: crate::config::VERSION.into(),
        update: None,
        updated_at: now_unix(),
    };
    let request = PairRequest { format: crate::config::Format::BeansV2, machine_pubkey: machine.pubkey(), box_pubkey: machine.box_pubkey(), device: device.clone() };
    let sealed = crate::crypto::seal_json(&ek, &request)?;
    // A mailbox that is gone means A cancelled, or the code expired. One that already holds
    // a request was used by a Device already: a code is good for one pairing.
    let gone = |error: crate::relay::RelayError| match error.status {
        Some(404) => anyhow::anyhow!("The other Device stopped waiting on this code. Get a fresh one from it."),
        Some(409) => anyhow::anyhow!("This code was already used. Each code pairs one Device; get a fresh one from the other Device."),
        _ => anyhow::anyhow!("{error}"),
    };
    app.relay.pair_post_request(&relay_url, &nonce, &sealed).await.map_err(gone)?;
    app.emit(Event::PairPosted { nonce: nonce.clone() });

    let reply: PairReply = tokio::time::timeout(PAIR_TIMEOUT, async {
        loop {
            match app.relay.pair_get_reply(&relay_url, &nonce).await {
                Ok(Some(sealed)) => return crate::crypto::unseal_json::<PairReply>(&machine.box_secret, &sealed),
                Ok(None) => tokio::time::sleep(POLL).await,
                Err(error) => return Err(gone(error)),
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("The other Device did not answer in time"))??;

    if reply.identity_pubkey != identity_pubkey {
        anyhow::bail!("The reply came from a different identity than the pairing string");
    }
    let dek = keys::unb64_32(&reply.account_dek)?;
    let _admission = app.plugin_admission(Some(incarnation)).map_err(anyhow::Error::msg)?;
    if app.has_identity() { anyhow::bail!("This Device already has an identity."); }

    let machine_file = MachineFile { format: crate::config::Format::BeansV2, machine_secret: keys::b64(&machine.secret),
    identity_pubkey: reply.identity_pubkey.clone(),
    content_pubkey: reply.content_pubkey.clone(),
    account_dek: keys::b64(&dek),
    name: device.name.clone(),
    os: device.os.clone(),
    os_version: device.os_version.clone(),
    model: device.model.clone(),
    registered: true,
    relay_url: Some(reply.relay_url.clone()),
    created_at: now_unix(), };
    *app.machine.lock().unwrap() = Some(machine_file);
    app.save_machine()?;
    app.set_relay_url(Some(reply.relay_url.clone()))?;
    {
        let mut state = app.state.lock().unwrap();
        *state = Default::default();
        upsert_device(&mut state.devices, device.clone());
    }
    app.store.clear()?;
    app.save_state();
    app.push_machine_blob_if_changed();
    app.emit(Event::IdentityChanged { has_identity: true });
    app.emit(Event::Snapshot(app.snapshot()));
    Ok(json!({ "id": device.id, "name": device.name, "os": device.os, "identity_id": keys::identity_id(&reply.identity_pubkey) }))
}

#[cfg(test)]
mod publication_tests {
    use super::*;

    #[tokio::test]
    async fn join_publication_rejects_forget_and_competing_create() {
        use axum::{routing::{get, post}, Json, Router};
        for transition in ["none", "forget", "create"] {
            let home = std::env::temp_dir().join(format!("beans-join-fence-{}", uuid::Uuid::new_v4()));
            let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
            let identity = crate::keys::Identity::generate();
            let ephemeral = crypto_box::SecretKey::generate(&mut rand::rngs::OsRng);
            let ek = keys::b64(ephemeral.public_key().as_bytes());
            let (arrived, waiting) = tokio::sync::oneshot::channel();
            let (release, blocked) = tokio::sync::oneshot::channel();
            let arrived = Arc::new(std::sync::Mutex::new(Some(arrived)));
            let blocked = Arc::new(tokio::sync::Mutex::new(Some(blocked)));
            let request = Arc::new(std::sync::Mutex::new(None::<PairRequest>));
            let posted = request.clone();
            let (pubkey, content) = (identity.pubkey(), identity.content_pubkey());
            let router = Router::new()
                .route("/v1/health", get(|| async { Json(json!({"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":1})) }))
                .route("/v1/pair/synthetic/request", post(move |Json(body): Json<Value>| {
                    let posted = posted.clone();
                    let secret = ephemeral.clone();
                    async move {
                        let bytes = keys::unb64(body["ciphertext"].as_str().unwrap()).unwrap();
                        *posted.lock().unwrap() = Some(crate::crypto::unseal_json(&secret, &bytes).unwrap());
                        Json(json!({}))
                    }
                }))
                .route("/v1/pair/synthetic/reply", get(move || {
                    let (arrived, blocked, request, pubkey, content) = (arrived.clone(), blocked.clone(), request.clone(), pubkey.clone(), content.clone());
                    async move {
                        arrived.lock().unwrap().take().unwrap().send(()).unwrap();
                        blocked.lock().await.take().unwrap().await.unwrap();
                        let request = request.lock().unwrap().take().unwrap();
                        let reply = json!({"format":"beans-v2","identity_pubkey":pubkey,"content_pubkey":content,"account_dek":keys::b64(&[42;32]),"relay_url":"http://127.0.0.1:1"});
                        let sealed = crate::crypto::seal_json(&request.box_pubkey, &reply).unwrap();
                        Json(json!({"ciphertext":keys::b64(&sealed)}))
                    }
                }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let held = app.clone();
            let joining = tokio::spawn(async move { join(&held, (url, identity.pubkey(), ek, "synthetic".into()), Some("Synthetic joined".into())).await });
            waiting.await.unwrap();
            let competing = match transition {
                "forget" => { app.forget_identity().unwrap(); None },
                "create" => { crate::identity::create(&app, Some("Synthetic replacement".into())).unwrap(); app.this_device_id() },
                _ => None,
            };
            release.send(()).unwrap();
            let result = joining.await.unwrap();
            match transition {
                "none" => { result.unwrap(); assert_eq!(app.machine_file().unwrap().name, "Synthetic joined"); },
                "forget" => { assert!(result.is_err()); assert!(app.machine_file().is_none()); assert!(!app.config.machine_path().exists()); },
                _ => { assert!(result.is_err()); assert_eq!(app.this_device_id(), competing); },
            }
            server.abort();
            drop(app);
            std::fs::remove_dir_all(home).unwrap();
        }
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;

    #[test]
    fn only_exact_versioned_pair_codes_with_unique_fields_are_accepted() {
        let key = keys::b64(&[7; 32]);
        let good = format!("beans://pair?v=2&relay=http%3A%2F%2F127.0.0.1%3A8787&id={key}&ek={key}&n=nonce");
        let parsed = parse_pairing_string(&good).unwrap();
        assert_eq!(parsed, ("http://127.0.0.1:8787".into(), key.clone(), key, "nonce".into()));
        for bad in [
            good.replace("beans://", "other://"), good.replace("v=2&", ""),
            good.replace("v=2", "v=1"), format!("{good}&v=2"), format!("{good}&n=other"),
            good.replace("n=nonce", "n=%GG"), good.replace("n=nonce", "n=%FF"),
            good.replace("relay=http%3A%2F%2F127.0.0.1%3A8787", "relay=file%3A%2F%2Faccount"),
        ] {
            assert!(parse_pairing_string(&bad).is_err(), "{bad}");
        }
    }
}
