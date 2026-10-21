use std::{process::{Child, Command, Stdio}, sync::Arc, time::Duration};

use beans::{app::{App, OutboxItem}, config::Config, keys::Machine, relay::BlobIn};
use serde_json::{json, Value};
use tokio::time::{sleep, timeout};

// Central QA supplies an already-built real relay binary. Never build or use a live relay here.
struct RelayProcess(Child);
impl Drop for RelayProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn outbox(app: &App) -> Value {
    let db = rusqlite::Connection::open_with_flags(
        app.config.database_path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).unwrap();
    let mut query = db.prepare(
        "SELECT id, kind, recipient, ciphertext, slot_name, slot_keep_first, group_name FROM outbox ORDER BY position",
    ).unwrap();
    let rows = query.query_map([], |row| Ok(json!([
        row.get::<_, String>(0)?, row.get::<_, String>(1)?,
        row.get::<_, Option<String>>(2)?, row.get::<_, Vec<u8>>(3)?,
        row.get::<_, Option<String>>(4)?, row.get::<_, bool>(5)?,
        row.get::<_, Option<String>>(6)?,
    ]))).unwrap().collect::<rusqlite::Result<Vec<Value>>>().unwrap();
    json!(rows)
}

fn queued_request(app: &App) -> Option<OutboxItem> {
    let db = rusqlite::Connection::open_with_flags(
        app.config.database_path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).unwrap();
    let mut query = db.prepare("SELECT id, recipient, ciphertext FROM outbox WHERE kind = 'request'").unwrap();
    let mut rows = query.query([]).unwrap();
    let row = rows.next().unwrap()?;
    let item = OutboxItem { id: row.get(0).unwrap(), kind: "request".into(),
        recipient: row.get(1).unwrap(), ciphertext: row.get(2).unwrap(), slot: None, group: None };
    assert!(rows.next().unwrap().is_none(), "scenario queues exactly one request");
    Some(item)
}

#[tokio::test]
async fn queued_request_from_revoked_device_cannot_write_surviving_runner_memory() {
    let binary = std::env::var_os("BEANS_REVOCATION_RELAY_BIN")
        .expect("central Rust QA must supply an already-built beans-relay binary");
    assert!(std::env::var_os("BEANS_RELAY_URL").is_none(), "fixture must not inherit a relay override");
    let scratch = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let url = format!("http://{address}");
    let mut command = Command::new(binary);
    command.env_clear();
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    let _relay = RelayProcess(command.arg("--bind").arg(address.to_string())
        .arg("--db").arg(scratch.path().join("relay.sqlite3"))
        .arg("--files-dir").arg(scratch.path().join("relay-files"))
        .stdout(Stdio::null()).stderr(Stdio::inherit()).spawn().unwrap());
    let runner = App::load(Config { home: scratch.path().join("runner"), port: 0 }).unwrap();
    let requester = App::load(Config { home: scratch.path().join("requester"), port: 0 }).unwrap();
    runner.settings.lock().unwrap().relay_url = Some(url.clone());
    requester.settings.lock().unwrap().relay_url = Some(url.clone());
    beans::identity::create(&runner, Some("surviving Runner".into())).unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            if runner.relay.health(&url).await.is_ok() { break; }
            sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("scratch relay did not become ready");
    beans::sync::ensure_registered(&runner, &url).await.unwrap();
    let runner_file = runner.machine_file().unwrap();
    let runner_machine = runner_file.machine().unwrap();
    let runner_id = runner_machine.pubkey();
    let runner_token = runner.relay.token(&url, &runner_machine).await.unwrap();
    let requester_machine = Machine::generate();
    let requester_id = requester_machine.pubkey();
    runner.relay.attest(&url, &runner_token, &requester_id, &requester_machine.box_pubkey()).await.unwrap();
    // Synthetic paired home receives the same account record and its own newly generated key.
    let mut requester_file = runner_file.clone();
    requester_file.machine_secret = beans::keys::b64(&requester_machine.secret);
    requester_file.name = "requester".into();
    requester_file.registered = true;
    requester_file.relay_url = Some(url.clone());
    *requester.machine.lock().unwrap() = Some(requester_file);
    requester.save_machine().unwrap();
    let runner_device = runner.local_device().unwrap();
    let requester_device = requester.local_device().unwrap();
    {
        let mut state = runner.state.lock().unwrap();
        state.devices.push(requester_device);
        state.listed_machines.insert(requester_id.clone(), 1);
    }
    {
        let mut state = requester.state.lock().unwrap();
        state.devices.push(runner_device);
        state.device_online.insert(runner_id.clone());
    }
    runner.save_state_now();
    requester.save_state_now();
    let bot_id = runner.state.lock().unwrap().bots[0].id.clone();
    beans::requests::memory_write(&runner, &bot_id, "retained memory\n", None).unwrap();
    let before = beans::requests::memory_read(&runner, &bot_id).unwrap();
    let hash = before["index"]["hash"].as_str().unwrap().to_owned();
    let asking = {
        let requester = Arc::clone(&requester);
        let runner_id = runner_id.clone();
        let bot_id = bot_id.clone();
        tokio::spawn(async move {
            beans::requests::ask_within(&requester, &runner_id, "memory.write",
                json!({"bot_id": bot_id, "text": "revoked requester mutation\n", "expected_hash": hash}),
                Duration::from_secs(60)).await
        })
    };
    let request = timeout(Duration::from_secs(5), async {
        loop {
            if let Some(item) = queued_request(&requester) { break item; }
            tokio::task::yield_now().await;
        }
    }).await.expect("public ask did not persist its request");
    let requester_token = requester.relay.token(&url, &requester_machine).await.unwrap();
    requester.relay.put_blob(&url, &requester_token, request.clone(), 0).await.unwrap();
    let (stored, _) = runner.relay.list_blobs(&url, &runner_token, 0, "request").await.unwrap();
    assert_eq!(stored.len(), 1);
    let queued: BlobIn = stored[0].clone();
    assert_eq!(queued.id, request.id);
    assert_eq!(queued.recipient_machine_pubkey.as_deref(), Some(runner_id.as_str()));
    assert_eq!(beans::keys::unb64(&queued.ciphertext).unwrap(), request.ciphertext);
    // Hold dispatch by leaving the actual encrypted envelope queued on the real relay.
    beans::sync::unpair_device(&runner, &requester_id).await.unwrap();
    runner.save_state_now();
    assert!(runner.device(&requester_id).is_none());
    assert!(!runner.state.lock().unwrap().listed_machines.contains_key(&requester_id));
    let db = rusqlite::Connection::open_with_flags(runner.config.database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let devices: Vec<String> = db.prepare("SELECT id FROM devices ORDER BY id").unwrap()
        .query_map([], |row| row.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(devices, vec![runner_id]);
    drop(db);
    let (still_queued, _) = runner.relay.list_blobs(&url, &runner_token, 0, "request").await.unwrap();
    assert_eq!(still_queued.iter().map(|blob| blob.id.as_str()).collect::<Vec<_>>(), vec![request.id.as_str()]);
    assert!(requester.relay.authenticate(&url, &requester_machine).await.unwrap_err().is_unpaired());
    // This fixture directly admits one delivery after the authoritative unpair completes.
    runner.state.lock().unwrap().caught_up = true;
    let runner_outbox_before = outbox(&runner);
    let requester_outbox_before = outbox(&requester);
    beans::sync::apply_blob(&runner, &runner_file, &still_queued[0]);
    // Rejected signed deliveries retain durable custody and relay bytes, never dispatch.
    let custody = rusqlite::Connection::open_with_flags(runner.config.database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let reason: String = custody.query_row("SELECT reason FROM transport_custody WHERE account=?1 AND id=?2", [&runner_file.identity_pubkey, &request.id], |row| row.get(0)).unwrap();
    assert_eq!(reason, "transport_rejected");
    drop(custody);
    let retained = runner.relay.list_blobs(&url, &runner_token, 0, "request").await.unwrap().0;
    assert_eq!(retained.iter().map(|blob| &blob.id).collect::<Vec<_>>(), vec![&request.id]);
    asking.abort();
    assert!(asking.await.unwrap_err().is_cancelled());
    let after = beans::requests::memory_read(&runner, &bot_id).unwrap();
    let memory_path = runner.config.home.join("workspaces").join(&bot_id).join("MEMORY.md");
    let stored_memory = std::fs::read_to_string(memory_path).unwrap();
    assert_eq!(outbox(&runner), runner_outbox_before, "revoked requester must receive no response or new Runner outbox effects");
    assert_eq!(outbox(&requester), requester_outbox_before, "requester's persisted intent remains exact");
    assert_eq!((after, stored_memory), (before, "retained memory\n".into()),
        "rejecting a request queued before revocation must not mutate surviving Runner memory after prune");
    // Exercise inner attribution inside authenticated version-1 transport, with a current peer.
    let current = Machine::generate();
    let current_id = current.pubkey();
    runner.relay.attest(&url, &runner_token, &current_id, &current.box_pubkey()).await.unwrap();
    {
        let mut state = runner.state.lock().unwrap();
        state.listed_machines.insert(current_id.clone(), 1);
        state.listed_machine_boxes.insert(current_id.clone(), current.box_pubkey());
        state.devices.push(beans::model::Device { id: current_id.clone(), name: "current requester".into(),
            box_pubkey: current.box_pubkey(), os: "ios".into(), envelope_version: Some(1), ..Default::default() });
    }
    for mode in ["unsigned", "forged", "valid"] {
        let mut candidate = beans::model::Request { id: format!("attribution-{mode}"), verb: "memory.write".into(),
            requested_by: current_id.clone(), created_at: beans::config::now_secs(),
            body: json!({"bot_id": bot_id, "text": "current signed mutation\n"}) };
        if mode != "unsigned" {
            let signing_bytes = serde_json::to_vec(&("beans-request-v1", &candidate.id, &candidate.verb,
                &candidate.requested_by, runner.this_device_id().unwrap(), candidate.created_at, &candidate.body)).unwrap();
            let signer = if mode == "forged" { &requester_machine } else { &current };
            candidate.body = json!({"payload": candidate.body, "signature": signer.sign(&signing_bytes)});
        }
        let envelope = json!({"envelope_version":1, "identity":runner_file.identity_pubkey,
            "from":current_id, "to":runner.this_device_id().unwrap(), "kind":"request",
            "id":uuid::Uuid::new_v4().to_string(), "at":beans::config::now_unix(), "body":candidate});
        let bytes = serde_json::to_vec(&envelope).unwrap();
        let signed = json!({"envelope_version":1, "envelope":beans::keys::b64(&bytes),
            "signature":current.sign(&[b"beans/envelope/v1\0".as_slice(), &bytes].concat())});
        let ciphertext = beans::crypto::seal_json(&runner_machine.box_pubkey(), &signed).unwrap();
        let item = OutboxItem { id: candidate.id.clone(), kind: "request".into(), recipient: runner.this_device_id(),
            ciphertext, slot: None, group: None };
        let current_token = runner.relay.authenticate(&url, &current).await.unwrap();
        runner.relay.put_blob(&url, &current_token, item, 0).await.unwrap();
        let (pending, _) = runner.relay.list_blobs(&url, &runner_token, 0, "request").await.unwrap();
        let delivery = pending.iter().find(|blob| blob.id == candidate.id).unwrap();
        beans::sync::apply_blob(&runner, &runner_file, delivery);
        if mode == "valid" {
            timeout(Duration::from_secs(5), async {
                loop {
                    let db = rusqlite::Connection::open_with_flags(runner.config.database_path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
                    let operation = format!("request:{}:{}", current_id, candidate.id);
                    let complete: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM transport_operations WHERE account=?1 AND id=?2 AND status='completed')", [&runner_file.identity_pubkey, &operation], |row| row.get(0)).unwrap();
                    if complete { break; }
                    sleep(Duration::from_millis(10)).await;
                }
            }).await.expect("valid attribution request did not finish");
        } else {
            let db = rusqlite::Connection::open_with_flags(runner.config.database_path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let reason: String = db.query_row("SELECT reason FROM transport_custody WHERE account=?1 AND id=?2", [&runner_file.identity_pubkey, &candidate.id], |row| row.get(0)).unwrap();
            assert_eq!(reason, "transport_rejected");
        }
        let memory = std::fs::read_to_string(runner.config.home.join("workspaces").join(&bot_id).join("MEMORY.md")).unwrap();
        assert_eq!(memory, if mode == "valid" { "current signed mutation\n" } else { "retained memory\n" }, "{mode} attribution effect");
        let db = rusqlite::Connection::open_with_flags(runner.config.database_path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let replies: Vec<Vec<u8>> = db.prepare("SELECT ciphertext FROM outbox WHERE kind = 'response' AND recipient = ?1 ORDER BY position").unwrap()
            .query_map([&current_id], |row| row.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        if mode == "valid" {
            let signed: Value = beans::crypto::unseal_json(&current.box_secret, replies.last().unwrap()).unwrap();
            let envelope: Value = serde_json::from_slice(&beans::keys::unb64(signed["envelope"].as_str().unwrap()).unwrap()).unwrap();
            let response: beans::model::Response = serde_json::from_value(envelope["body"].clone()).unwrap();
            assert_eq!(response.request_id, candidate.id);
            assert!(response.error.is_none());
            assert_eq!(response.body["hash"], beans::requests::memory_read(&runner, &bot_id).unwrap()["index"]["hash"]);
        } else {
            assert!(replies.is_empty(), "rejected attribution must not produce a response");
        }
    }
}
