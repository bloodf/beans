use super::*;

#[test]
fn signed_library_confirmation_boundary() {
    use std::future::Future;
    use std::task::Poll;
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().max_blocking_threads(1).build().unwrap();
    runtime.block_on(async {
        let home = std::env::temp_dir().join(format!("beans-library-consumer-{}", uuid::Uuid::new_v4()));
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
        let selected = home.join("synthetic-selected");
        std::fs::create_dir(&selected).unwrap();
        let skill = "---\nname: Fixture\ndescription: Exact inactive copy\nlicense: MIT\nhooks: forbidden\n---\nNEVER_EXECUTE\n";
        std::fs::write(selected.join("SKILL.md"), skill).unwrap();
        std::fs::write(selected.join("resource"), b"abc").unwrap();
        let make = |verb: &str, body: Value| {
            let mut request = Request { id: format!("synthetic-{}", uuid::Uuid::new_v4()), verb: verb.into(), requested_by: phone.pubkey(), body: body.clone(), created_at: now_secs() };
            request.body = json!({"payload":body,"signature":phone.sign(&request_bytes(&request,&runner,&body).unwrap())});
            request
        };
        let preview_request = || make("skills.library.preview", json!({"version":1,"runner_id":runner,"root":selected}));
        let preview = answer(&app, &preview_request()).await.unwrap();
        assert_eq!(preview["summary"]["name"], "Fixture");
        assert_eq!(preview["summary"]["license"], "MIT");
        assert_eq!(preview["summary"]["file_count"], 2);
        let owned = home.join("skill-library");
        assert_eq!(std::fs::read_dir(&owned).unwrap().count(), 0, "Preview must not stage or publish");
        let import_body = |token: &Value, confirmed| json!({"version":1,"runner_id":runner,"token":token,"confirmed":confirmed});
        assert_eq!(answer(&app, &make("skills.library.import", import_body(&preview["token"], false))).await.unwrap_err(), "skill_library_confirmation_required");
        std::fs::write(selected.join("resource"), b"changed").unwrap();
        assert_eq!(answer(&app, &make("skills.library.import", import_body(&preview["token"], true))).await.unwrap_err(), "skill_library_source_changed");
        assert_eq!(std::fs::read_dir(&owned).unwrap().count(), 0);
        assert_eq!(answer(&app, &make("skills.library.import", import_body(&preview["token"], true))).await.unwrap_err(), "skill_library_approval_not_found");
        std::fs::write(selected.join("resource"), b"abc").unwrap();
        let preview = answer(&app, &preview_request()).await.unwrap();
        let mut forged = make("skills.library.import", import_body(&preview["token"], true));
        forged.body["payload"]["token"] = json!("0".repeat(32));
        assert_eq!(answer(&app, &forged).await.unwrap_err(), "Invalid request signature");
        let imported = answer(&app, &make("skills.library.import", import_body(&preview["token"], true))).await.unwrap();
        let id = imported["managed_id"].as_str().unwrap();
        assert_eq!(std::fs::read(owned.join(id).join("content/resource")).unwrap(), b"abc");
        let manifest: Value = serde_json::from_slice(&std::fs::read(owned.join(id).join("manifest.json")).unwrap()).unwrap();
        assert!(manifest["files"].as_array().unwrap().iter().any(|f| f["path"] == "resource" && f["sha256"] == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
        std::fs::write(owned.join("unrelated"), b"keep").unwrap();
        let removal = |managed_id: &str, confirmed| make("skills.library.uninstall", json!({"version":1,"runner_id":runner,"managed_id":managed_id,"confirmed":confirmed}));
        assert_eq!(answer(&app, &removal(id, false)).await.unwrap_err(), "skill_library_confirmation_required");
        assert_eq!(answer(&app, &removal("../synthetic-selected", true)).await.unwrap_err(), "invalid_skill_library_request");
        let removed = answer(&app, &removal(id, true)).await.unwrap();
        assert_eq!(removed["managed_id"], id);
        assert!(!owned.join(id).exists());
        assert_eq!(std::fs::read(owned.join("unrelated")).unwrap(), b"keep");
        assert_eq!(std::fs::read_to_string(selected.join("SKILL.md")).unwrap(), skill);
        assert_eq!(std::fs::read(selected.join("resource")).unwrap(), b"abc");
        assert!(!selected.join("marker").exists());
        // Queue import behind one worker; revoke actor after async admission, before effect.
        let preview = answer(&app, &preview_request()).await.unwrap();
        let request = make("skills.library.import", import_body(&preview["token"], true));
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
        assert_eq!(std::fs::read_dir(&owned).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>(), vec![std::ffi::OsString::from("unrelated")]);
        drop(app);
    });
}
