use super::*;

#[test]
fn signed_discovery_rechecks_revocation_after_scan() {
    use std::future::Future;
    use std::task::Poll;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().max_blocking_threads(1).build().unwrap();
    runtime.block_on(async {
        let home = std::env::temp_dir().join(format!("beans-discovery-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        let root = home.join("selected");
        std::fs::create_dir_all(&root).unwrap();
        let source = "---\nname: Fixture\ndescription: Selected metadata\nlicense: MIT\nhooks: forbidden\n---\nBODY_PRIVATE\n";
        std::fs::write(root.join("SKILL.md"), source).unwrap();
        let hook = root.join("hook.sh");
        std::fs::write(&hook, "#!/bin/sh\ntouch marker\n").unwrap();
        crate::identity::create(&app, Some("Synthetic Runner".into())).unwrap();
        let runner = app.this_device_id().unwrap();
        let phone = crate::keys::Machine::generate();
        {
            let mut state = app.state.lock().unwrap();
            state.devices.push(crate::model::Device { id: phone.pubkey(), name: "Synthetic phone".into(), os: "ios".into(), box_pubkey: phone.box_pubkey(), ..Default::default() });
            state.listed_machines.insert(phone.pubkey(), 1);
            state.listed_machines.insert(runner.clone(), 1);
        }
        let body = json!({"version":1,"runner_id":runner,"root":root});
        let mut request = Request { id: "synthetic-scan".into(), verb: "skills.discovery".into(), requested_by: phone.pubkey(), body: body.clone(), created_at: now_secs() };
        request.body = json!({"payload":body,"signature":phone.sign(&request_bytes(&request,&runner,&body).unwrap())});
        let positive = answer(&app, &request).await.unwrap();
        assert_eq!(positive["status"], "scanned", "Fixture requires Linux with usable openat2");
        assert_eq!(positive["skills"], json!([{"path":"SKILL.md","name":"Fixture","description":"Selected metadata","license":"MIT"}]));
        assert!(positive["diagnostics"].as_array().unwrap().iter().any(|d| d["code"] == "unsupported_metadata"));
        assert!(!positive.to_string().contains("BODY_PRIVATE"));
        // Occupy the only blocking worker so revocation occurs after admission, before scan completion.
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || { started_tx.send(()).unwrap(); release_rx.recv().unwrap(); });
        started_rx.recv().unwrap();
        let mut pending = Box::pin(answer(&app, &request));
        std::future::poll_fn(|cx| { assert!(matches!(pending.as_mut().poll(cx), Poll::Pending)); Poll::Ready(()) }).await;
        app.state.lock().unwrap().listed_machines.remove(&phone.pubkey());
        release_tx.send(()).unwrap();
        assert_eq!(pending.await.unwrap_err(), "skill_discovery_authority_changed");
        blocker.await.unwrap();
        assert!(!root.join("marker").exists());
        assert_eq!(std::fs::read_to_string(root.join("SKILL.md")).unwrap(), source);
        assert_eq!(std::fs::read_to_string(hook).unwrap(), "#!/bin/sh\ntouch marker\n");
        drop(app);
        std::fs::remove_dir_all(home).unwrap();
    });
}
