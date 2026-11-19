//! Execution authority: fences, leases, recovery, and schema upgrades.

use super::*;

#[test]
fn task_duplicate_cancel_and_prune_keep_replay_denial() {
    let scratch = scratch(); let store = &scratch.0;
    let (lease,job) = task_fixture(store,"job-test");
    assert!(store.queue_task(&lease,&job).unwrap());
    assert!(!store.queue_task(&lease,&job).unwrap());
    store.cancel_tasks(&lease.account_epoch,Some(&job.id),None,None).unwrap();
    assert!(!store.start_task(&lease).unwrap());
    store.recover_task_owner("owner-test").unwrap();
    assert_eq!(store.task_state(&lease).unwrap(),Some(TaskState::Interrupted));
    assert!(store.resolve_task(&lease,0).unwrap());
    store.prune_tasks(30*24*60*60+1).unwrap();
    assert_eq!(store.task_state(&lease).unwrap(),None);
    assert!(!store.queue_task(&lease,&job).unwrap());
    let (cancelled,new_job) = task_fixture(store,"job-cancel-first");
    store.cancel_tasks(&cancelled.account_epoch,Some(&new_job.id),None,None).unwrap();
    assert!(!store.queue_task(&cancelled,&new_job).unwrap());
}

#[test]
fn abandoned_queued_task_is_resolvable_without_recovery_and_keeps_fence() {
    let scratch = scratch();
    let store = &scratch.0;
    let (lease, mut job) = task_fixture(store, "job-abandoned");
    job.routine_id = Some("routine-test".into());
    assert!(store.queue_task(&lease, &job).unwrap());
    let mut stale = lease.clone();
    stale.execution_id = "different-execution".into();
    assert!(!store.abandon_queued_task(&stale).unwrap());
    assert_eq!(store.task_state(&lease).unwrap(), Some(TaskState::Queued));
    store.cancel_tasks(&lease.account_epoch, Some(&job.id), None, None).unwrap();
    assert!(!store.start_task(&lease).unwrap());
    assert!(store.abandon_queued_task(&lease).unwrap());
    assert_eq!(store.task_state(&lease).unwrap(), Some(TaskState::Interrupted));
    assert!(store.routine_needs_review(&lease.account_epoch, "routine-test").unwrap());
    assert!(store.resolve_task(&lease, 1).unwrap());
    store.safety(|tx| {
        assert!(!schedule_unresolved_tx(tx, &lease.account_epoch, "routine-test")?);
        Ok(())
    }).unwrap();
    assert!(!store.queue_task(&lease, &job).unwrap());
    assert!(!store.abandon_queued_task(&lease).unwrap());
    let (running, job) = task_fixture(store, "job-running");
    assert!(store.queue_task(&running, &job).unwrap());
    assert!(store.start_task(&running).unwrap());
    assert!(!store.abandon_queued_task(&running).unwrap());
    assert_eq!(store.task_state(&running).unwrap(), Some(TaskState::Running));

    let (uncertain, mut job) = task_fixture(store, "job-uncertain");
    job.routine_id = Some("routine-uncertain".into());
    assert!(store.queue_task(&uncertain, &job).unwrap());
    store.connection.lock().unwrap().execute(
        "INSERT INTO task_effect_receipts VALUES(?1,?2,?3,?4,'attempt-uncertain','receipt-uncertain','unknown')",
        params![uncertain.account_epoch, uncertain.task_id, uncertain.owner_epoch, uncertain.execution_id],
    ).unwrap();
    assert!(store.abandon_queued_task(&uncertain).unwrap());
    assert_eq!(store.task_state(&uncertain).unwrap(), Some(TaskState::NeedsReview));
    assert!(store.resolve_task(&uncertain, 1).unwrap());
    store.safety(|tx| {
        assert!(schedule_unresolved_tx(tx, &uncertain.account_epoch, "routine-uncertain")?);
        let receipt: (String, String, String, String) = tx.query_row(
            "SELECT owner_epoch,execution_id,attempt_id,state FROM task_effect_receipts WHERE account_epoch=?1 AND receipt_id='receipt-uncertain'",
            [&uncertain.account_epoch], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(receipt, (uncertain.owner_epoch.clone(), uncertain.execution_id.clone(), "attempt-uncertain".into(), "unknown".into()));
        Ok(())
    }).unwrap();
    assert!(!store.queue_task(&uncertain, &job).unwrap());
}

#[test]
fn task_recovery_unknown_is_immutable_and_old_owner_is_stale() {
    let scratch = scratch(); let store = &scratch.0;
    let (lease,job) = task_fixture(store,"job-test"); let binding=invocation();
    assert!(store.queue_task(&lease,&job).unwrap()); assert!(store.start_task(&lease).unwrap());
    assert!(store.insert_invocation(&lease,&binding).unwrap());
    assert!(store.decide_invocation(&lease,&binding,AuthorizationDecision::Authorized,AuthorizationKind::Rule,None,None,0).unwrap());
    assert!(store.admit_invocation(&lease,&binding).unwrap());
    assert!(!store.admit_invocation(&lease,&binding).unwrap());
    assert!(store.finish_receipt(&lease,&binding,ReceiptOutcome::Unknown).unwrap());
    assert!(!store.finish_receipt(&lease,&binding,ReceiptOutcome::Finished).unwrap());
    store.recover_task_owner("owner-replacement").unwrap();
    assert_eq!(store.task_state(&lease).unwrap(),Some(TaskState::NeedsReview));
    assert!(store.finish_receipt(&lease,&binding,ReceiptOutcome::Finished).is_err());
    store.close_task_account().unwrap(); store.clear().unwrap();
    assert!(store.finish_receipt(&lease,&binding,ReceiptOutcome::Finished).is_err());
    let count:i64=store.connection.lock().unwrap().query_row("SELECT count(*) FROM task_effect_receipts",[],|r|r.get(0)).unwrap();
    assert_eq!(count,0);
}

#[test]
fn routine_check_and_task_insert_roll_back_together() {
    let scratch = scratch(); let store=&scratch.0;
    let (lease,mut job)=task_fixture(store,"job-check"); job.routine_id=Some("routine-test".into());
    let set=std::collections::BTreeMap::from([("seen".into(),serde_json::json!(true))]);
    store.connection.lock().unwrap().execute_batch("CREATE TRIGGER refuse_check BEFORE INSERT ON routine_check_commits BEGIN SELECT RAISE(ABORT,'synthetic rejection'); END;").unwrap();
    assert!(store.commit_routine_check(&lease,Some(&job),"routine-test",&job.chat_id,&job.bot_id,1,&set,&[]).is_err());
    assert_eq!(store.task_state(&lease).unwrap(),None);
    assert!(!store.codemode_values(&job.chat_id,&job.bot_id).unwrap().contains_key("seen"));
    store.connection.lock().unwrap().execute_batch("DROP TRIGGER refuse_check;").unwrap();
    assert!(store.commit_routine_check(&lease,Some(&job),"routine-test",&job.chat_id,&job.bot_id,1,&set,&[]).unwrap());
    assert_eq!(store.task_state(&lease).unwrap(),Some(TaskState::Queued));
    assert_eq!(store.codemode_values(&job.chat_id,&job.bot_id).unwrap().get("seen"),Some(&serde_json::json!(true)));
}

#[test]
fn legacy_owner_migration_preserves_data_and_denies_prior_work() {
    let scratch=scratch(); let store=&scratch.0;
    store.upsert(&message("legacy-message",1.0)).unwrap();
    let item=OutboxItem { id:"legacy-outbox".into(),kind:"job".into(),recipient:Some("remote-runner".into()),ciphertext:vec![1,2,3],slot:None,group:None };
    store.queue_outbox_with_state(&item,&State::default()).unwrap();
    let before_state=store.load_state().unwrap();
    let before_outbox=store.last_outbox().unwrap().unwrap();
    let sent=SentJob { id:"job-legacy".into(),chat_id:"chat-test".into(),bot_id:"bot-test".into(),routine_id:None,runner_id:"runner-test".into(),sent_at:1.0 };
    store.insert_sent_job(&sent).unwrap();
    store.connection.lock().unwrap().execute("INSERT INTO memory_turn_admissions VALUES('job-memory','{}')",[]).unwrap();
    let before=store.message("chat","legacy-message").unwrap();
    let epoch=store.recover_task_owner("owner-test").unwrap();
    assert_eq!(store.message("chat","legacy-message").unwrap(),before);
    assert_eq!(store.load_state().unwrap().last_seq,before_state.last_seq);
    let after_outbox=store.last_outbox().unwrap().unwrap();
    assert_eq!(after_outbox.id,before_outbox.id);
    assert_eq!(after_outbox.ciphertext,before_outbox.ciphertext);
    assert_eq!(store.sent_jobs().unwrap(),vec![sent]);
    let (mut lease,mut job)=task_fixture(store,"job-legacy"); lease.account_epoch=epoch;
    job.created_at=crate::config::now_secs()+1.0;
    assert!(!store.queue_task(&lease,&job).unwrap(),"known legacy identity stays denied even with a new timestamp");
    lease.task_id="job-unseen-old".into(); job.id=lease.task_id.clone(); job.created_at=1.0;
    assert!(!store.queue_task(&lease,&job).unwrap());
    job.created_at=crate::config::now_secs()+1.0;
    assert!(!store.queue_task(&lease,&job).unwrap(),"denied legacy envelope has a lifetime fence");
    lease.task_id="job-explicit-new".into(); job.id=lease.task_id.clone();
    assert!(!store.queue_task(&lease,&job).unwrap(),"legacy-unseen execution stays closed without new-intent proof");
    assert_eq!(store.task_state(&lease).unwrap(),None);
}
#[test]
fn legacy_unseen_future_timestamp_replay_is_denied_across_reopen() {
    let home=tempfile::tempdir().unwrap(); let path=home.path().join("beans.sqlite3");
    let store=LocalStore::open(&path).unwrap();
    store.upsert(&message("legacy-transcript",1.0)).unwrap();
    let epoch=store.recover_task_owner("owner-test").unwrap();
    let (lease,mut job)=task_fixture(&store,"job-previously-executed-unseen");
    job.created_at=4_000_000_000.0;
    assert!(!store.queue_task(&lease,&job).unwrap());
    assert_eq!(store.task_state(&lease).unwrap(),None);
    drop(store);
    let reopened=LocalStore::open(&path).unwrap();
    assert_eq!(reopened.recover_task_owner("owner-test").unwrap(),epoch);
    assert!(!reopened.queue_task(&lease,&job).unwrap());
    assert_eq!(reopened.task_state(&lease).unwrap(),None);
}

#[test]
fn current_schema_missing_all_safety_tables_is_not_a_predecessor() {
    let home=tempfile::tempdir().unwrap(); let path=home.path().join("beans.sqlite3");
    let store=LocalStore::open(&path).unwrap();
    store.recover_task_owner("owner-test").unwrap();
    store.connection.lock().unwrap().execute_batch("DROP TABLE task_authority; DROP TABLE task_fences; DROP TABLE local_tasks; DROP TABLE task_invocations; DROP TABLE task_effect_receipts; DROP TABLE routine_check_commits; DROP TABLE task_safety_version;").unwrap();
    drop(store);
    assert!(LocalStore::open(&path).is_err());
}

#[test]
fn schema_three_upgrade_preserves_task_fences_and_adds_empty_quarantine() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("beans.sqlite3");
    let store = LocalStore::open(&path).unwrap();
    let (lease, job) = task_fixture(&store, "prior-fence");
    assert!(store.queue_task(&lease, &job).unwrap());
    store.connection.lock().unwrap().execute_batch("DROP TABLE task_rejected_envelopes; PRAGMA user_version=3;").unwrap();
    drop(store);
    let upgraded = LocalStore::open(&path).unwrap();
    assert!(!upgraded.queue_task(&lease, &job).unwrap());
    let count: i64 = upgraded.connection.lock().unwrap().query_row("SELECT count(*) FROM task_rejected_envelopes", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn missing_quarantine_schema_is_refused_without_reconstruction() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("beans.sqlite3");
    let store = LocalStore::open(&path).unwrap();
    store.connection.lock().unwrap().execute_batch("DROP TABLE task_rejected_envelopes;").unwrap();
    drop(store);
    assert!(LocalStore::open(&path).is_err());
    let connection = Connection::open(path).unwrap();
    let count: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE name='task_rejected_envelopes'", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn quarantine_and_owner_admission_serialize_without_duplicate_execution() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("beans.sqlite3");
    let store = LocalStore::open(&path).unwrap();
    let (lease, job) = task_fixture(&store, "racing-envelope");
    let reader = LocalStore::open(&path).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let other = barrier.clone();
    let reject = std::thread::spawn(move || {
        other.wait();
        reader.quarantine_task_envelope("account-identity", "racing-envelope").unwrap();
    });
    barrier.wait();
    let admitted = store.queue_task_for_identity(&lease, &job, "account-identity").unwrap();
    reject.join().unwrap();
    assert_eq!(store.task_state(&lease).unwrap().is_some(), admitted, "a later refusal cannot rewrite already committed admission");
    assert!(!store.queue_task_for_identity(&lease, &job, "account-identity").unwrap());
    let (first, first_job) = task_fixture(&store, "owner-first");
    assert!(store.queue_task_for_identity(&first, &first_job, "account-identity").unwrap());
    store.quarantine_task_envelope("account-identity", &first_job.id).unwrap();
    assert_eq!(store.task_state(&first).unwrap(), Some(TaskState::Queued));
    assert!(!store.queue_task_for_identity(&first, &first_job, "account-identity").unwrap());
    let (denied, job) = task_fixture(&store, "rejection-first");
    store.quarantine_task_envelope("account-identity", &job.id).unwrap();
    assert!(!store.queue_task_for_identity(&denied, &job, "account-identity").unwrap());
    assert_eq!(store.task_state(&denied).unwrap(), None);
    store.retain_chats(&[]).unwrap();
    store.prune_tasks(i64::MAX).unwrap();
    assert!(!store.queue_task_for_identity(&denied, &job, "account-identity").unwrap());
}

#[test]
fn future_task_schema_is_refused_without_downgrade() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("beans.sqlite3");
    let store = LocalStore::open(&path).unwrap();
    store.recover_task_owner("owner-test").unwrap();
    store.connection.lock().unwrap().execute_batch("PRAGMA user_version=5;").unwrap();
    drop(store);
    assert!(LocalStore::open(&path).is_err());
    let connection = Connection::open(&path).unwrap();
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
    assert_eq!(version, 5, "an older core must not silently downgrade a future safety schema");
}

#[test]
fn supported_predecessor_keeps_serving_data_without_execution() {
    let home=tempfile::tempdir().unwrap(); let path=home.path().join("beans.sqlite3");
    let store=LocalStore::open(&path).unwrap();
    store.upsert(&message("legacy-transcript",1.0)).unwrap();
    store.connection.lock().unwrap().execute_batch("DROP TABLE task_authority; DROP TABLE task_fences; DROP TABLE local_tasks; DROP TABLE task_invocations; DROP TABLE task_effect_receipts; DROP TABLE routine_check_commits; DROP TABLE task_safety_version; DROP TABLE routine_schedule_authority; DROP TABLE routine_schedule_occurrences; PRAGMA user_version=1;").unwrap();
    drop(store);
    let reopened=LocalStore::open(&path).unwrap();
    reopened.recover_task_owner("owner-test").unwrap();
    assert!(reopened.message("chat","legacy-transcript").unwrap().is_some());
    let (lease,mut job)=task_fixture(&reopened,"job-unseen"); job.created_at=4_000_000_000.0;
    assert!(!reopened.queue_task(&lease,&job).unwrap());
    reopened.close_task_account().unwrap(); reopened.clear().unwrap();
    reopened.task_account_epoch("owner-test").unwrap();
    let marked:bool=reopened.connection.lock().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM task_safety_version)",[],|r|r.get(0)).unwrap();
    assert!(marked);
}


#[test]
fn legacy_migration_failure_leaves_authority_and_journals_unchanged() {
    let scratch=scratch(); let store=&scratch.0;
    store.connection.lock().unwrap().execute("INSERT INTO device_turns VALUES('legacy-device','invalid-json')",[]).unwrap();
    assert!(store.recover_task_owner("owner-test").is_err());
    assert_eq!(store.current_task_account_epoch().unwrap(),None);
    let json:String=store.connection.lock().unwrap().query_row("SELECT json FROM device_turns WHERE id='legacy-device'",[],|r|r.get(0)).unwrap();
    assert_eq!(json,"invalid-json");
    let count:i64=store.connection.lock().unwrap().query_row("SELECT count(*) FROM task_fences",[],|r|r.get(0)).unwrap();
    assert_eq!(count,0);
}

#[test]
fn recovered_routine_resolution_keeps_unknown_and_replay_fence() {
    let scratch=scratch(); let store=&scratch.0;
    let (lease,mut job)=task_fixture(store,"job-resolve"); job.routine_id=Some("routine-test".into());
    assert!(store.queue_task(&lease,&job).unwrap()); assert!(store.start_task(&lease).unwrap());
    let binding=invocation();
    assert!(store.insert_invocation(&lease,&binding).unwrap());
    assert!(store.decide_invocation(&lease,&binding,AuthorizationDecision::Authorized,AuthorizationKind::Rule,None,None,1).unwrap());
    assert!(store.admit_invocation(&lease,&binding).unwrap());
    store.recover_task_owner("owner-new").unwrap();
    assert!(store.routine_needs_review(&lease.account_epoch,"routine-test").unwrap());
    let history=store.history_task_lease("owner-new",&job.id,0).unwrap().unwrap();
    assert!(store.resolve_task(&history,2).unwrap());
    assert!(!store.routine_needs_review(&lease.account_epoch,"routine-test").unwrap());
    assert!(store.finish_receipt(&lease,&binding,ReceiptOutcome::Finished).is_err());
    let state:String=store.connection.lock().unwrap().query_row("SELECT state FROM task_effect_receipts WHERE receipt_id=?1",[&binding.receipt_id],|r|r.get(0)).unwrap();
    assert_eq!(state,"unknown");
}
