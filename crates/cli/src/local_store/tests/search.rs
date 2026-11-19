//! Exact search: the projection, its triggers, and its backfill. Statements that stand in for
//! another binary's writes stay inline here on purpose: they are what the store must survive.

use super::*;

/// Messages whose `scope` holds every term of `query`, newest first, confirmed as `search`
/// confirms them.
fn found(store: &LocalStore, query: &str, scope: SearchScope) -> anyhow::Result<Vec<String>> {
    let terms = search_terms(query);
    let mut ids = Vec::new();
    store.visit_search_candidates(&terms, scope, |message| {
        let hit = match (scope, &message.body) {
            (SearchScope::Text, body) => search_body_text(body).is_some_and(|text| search_matches(text, &terms)),
            (SearchScope::Files, Body::Text { attachments, .. }) => attachments.iter().any(|a| search_matches(&a.name, &terms)),
            _ => false,
        };
        if hit {
            ids.push(message.id);
        }
        true
    })?;
    Ok(ids)
}

fn text(store: &LocalStore, query: &str) -> Vec<String> {
    found(store, query, SearchScope::Text).unwrap()
}

/// `(id, chat, position, stale, text, files, images)` for every projection row, by id.
type Row = (String, String, i64, i64, String, String, String);

fn rows(store: &LocalStore) -> Vec<Row> {
    let connection = store.connection.lock().unwrap();
    let mut statement = connection.prepare("SELECT message_id, chat_id, position, stale, text, files, images FROM message_search ORDER BY message_id").unwrap();
    statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

fn mirrors_messages(store: &LocalStore) -> bool {
    let connection = store.connection.lock().unwrap();
    connection
        .query_row(
            "SELECT (SELECT count(*) FROM messages) = (SELECT count(*) FROM message_search)
                AND NOT EXISTS(SELECT 1 FROM messages m LEFT JOIN message_search s ON s.message_id = m.id
                    WHERE s.message_id IS NULL OR s.chat_id IS NOT m.chat_id OR s.position IS NOT m.position OR s.created_at IS NOT m.created_at)",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

fn with_files(id: &str, at: f64, text: &str, files: &[(&str, &str, &str)]) -> Message {
    let mut message = message(id, at);
    message.body = Body::Text {
        text: text.into(),
        attachments: files.iter().map(|(id, name, mime)| crate::model::Attachment { id: (*id).into(), name: (*name).into(), mime: (*mime).into(), size: 1, width: None, height: None }).collect(),
        mentions: vec![],
        reply_to: None,
    };
    message
}

#[test]
fn the_matcher_folds_case_and_finds_substrings() {
    let terms = search_terms("release INFRA \" OR *");
    assert_eq!(terms, ["release", "infra", "or"]);
    assert!(search_matches("Release Room Infrastructure or not", &terms));
    assert!(search_snippet("Release Room Infrastructure", &terms).contains("Release"));
    let scratch = scratch();
    let mut deployed = message("deployed", 2.0);
    deployed.body = Body::text("Deployed the résumé service to 上海市");
    scratch.0.upsert(&deployed).unwrap();
    assert_eq!(text(&scratch.0, "ploy RÉSUMÉ 海"), ["deployed"]);
    deployed.body = Body::text("Finished something else");
    scratch.0.upsert(&deployed).unwrap();
    assert!(text(&scratch.0, "deploy").is_empty());
    scratch.0.remove("chat", "deployed").unwrap();
    assert!(text(&scratch.0, "finished").is_empty());
}

#[test]
fn only_prefilter_candidates_are_decoded() {
    let scratch = scratch();
    let store = &scratch.0;
    let mut hit = message("hit", 2.0);
    hit.body = Body::text("Meeting in 上海市 about the RÉSUMÉ");
    store.upsert(&hit).unwrap();
    store.upsert(&message("other", 1.0)).unwrap();
    // A row the prefilter excludes is never decoded: corrupt it behind a fresh index row.
    let connection = store.connection.lock().unwrap();
    connection.execute("UPDATE messages SET message_json = 'not json' WHERE id = 'other'", []).unwrap();
    connection.execute("UPDATE message_search SET stale = 0 WHERE message_id = 'other'", []).unwrap();
    drop(connection);
    assert_eq!(text(store, "海 résumé"), ["hit"]);
    assert!(found(store, "other", SearchScope::Text).is_err(), "a candidate is decoded and confirmed");
}

#[test]
fn attachment_names_and_images_are_projected_per_attachment() {
    let scratch = scratch();
    let store = &scratch.0;
    store.upsert(&with_files("m", 1.0, "see attached", &[("a", "Ledger 上海.XLSX", "application/vnd.ms-excel"), ("b", "Beach.JPG", "image/jpeg")])).unwrap();
    assert_eq!(found(store, "ledger xlsx", SearchScope::Files).unwrap(), ["m"]);
    assert!(found(store, "ledger beach", SearchScope::Files).unwrap().is_empty(), "terms in two names are no file hit");
    let row = &rows(store)[0];
    assert_eq!((row.5.as_str(), row.6.as_str()), ("ledger 上海.xlsx\nbeach.jpg", "b"));
}

#[test]
fn a_store_that_predates_the_index_is_backfilled_without_decoding_tool_rows() {
    let scratch = scratch();
    let path = scratch.1.join("beans.sqlite3");
    scratch.0.upsert(&with_files("text", 1.0, "Quarterly numbers", &[("file", "Ledger 上海.XLSX", "application/vnd.ms-excel")])).unwrap();
    let mut tool = message("tool", 2.0);
    tool.body = serde_json::from_value(serde_json::json!({"kind": "tool", "name": "bash", "summary": "quarterly", "detail": "", "is_running": false})).unwrap();
    scratch.0.upsert(&tool).unwrap();
    // Roll the file back to the pre-index schema, with a tool row a backfill must not decode.
    scratch
        .0
        .connection
        .lock()
        .unwrap()
        .execute_batch(
            "DROP TRIGGER message_search_insert; DROP TRIGGER message_search_update; DROP TRIGGER message_search_delete;
             DROP TABLE message_search; DELETE FROM message_search_version;
             UPDATE messages SET message_json = 'tool output that is not json' WHERE id = 'tool';",
        )
        .unwrap();
    let reopened = LocalStore::open(&path).unwrap();
    assert!(rows(&reopened).iter().all(|row| row.3 == 1 && row.4.is_empty()), "upgrade seeds stale rows without decoding");
    assert!(mirrors_messages(&reopened));
    assert_eq!(text(&reopened, "QUARTERLY"), ["text"]);
    assert_eq!(found(&reopened, "ledger 上海 xlsx", SearchScope::Files).unwrap(), ["text"]);
    let rows_now = rows(&reopened);
    assert_eq!((rows_now[0].3, rows_now[0].4.as_str(), rows_now[0].5.as_str()), (0, "quarterly numbers", "ledger 上海.xlsx"));
    assert_eq!((rows_now[1].3, rows_now[1].4.as_str()), (0, ""));
    // A new `SEARCH_INDEX_VERSION` rebuilds the derived tables and derives everything again.
    reopened.connection.lock().unwrap().execute("UPDATE message_search_version SET version = 0", []).unwrap();
    drop(reopened);
    let reopened = LocalStore::open(&path).unwrap();
    assert!(rows(&reopened).iter().all(|row| row.3 == 1));
    assert_eq!(text(&reopened, "numbers"), ["text"]);
    assert!(rows(&reopened).iter().all(|row| row.3 == 0));
}

#[test]
fn the_projection_follows_every_message_write() {
    let scratch = scratch();
    let store = &scratch.0;
    for (id, at) in [("m1", 1.0), ("m2", 2.0), ("m3", 3.0)] {
        store.upsert(&message(id, at)).unwrap();
    }
    assert!(rows(store).iter().all(|row| row.3 == 0));
    // An older page meets rows already here: positions move, stored text stays.
    let mut moved = message("m1", 1.0);
    moved.body = Body::text("text the store never keeps");
    store.insert_older(&[message("m0", 0.5), moved]).unwrap();
    assert!(mirrors_messages(store));
    assert!(text(store, "never keeps").is_empty());
    assert_eq!(text(store, "m0"), ["m0"]);
    // A binary that predates the index writes through plain SQL; the triggers keep up.
    let mut rewritten = message("m2", 2.0);
    rewritten.body = Body::text("Rewritten by an older Runner");
    let mut legacy = message("m4", 4.0);
    legacy.body = Body::text("Inserted by an older Runner");
    let connection = store.connection.lock().unwrap();
    connection.execute("UPDATE messages SET message_json = ?1 WHERE id = 'm2'", [serde_json::to_string(&rewritten).unwrap()]).unwrap();
    connection
        .execute(
            "INSERT INTO messages(id,chat_id,position,sort_at,created_at,author_kind,body_kind,is_complete,text_nonempty,message_json)
             VALUES('m4','chat',99,4.0,4.0,'you','text',1,1,?1)",
            [serde_json::to_string(&legacy).unwrap()],
        )
        .unwrap();
    drop(connection);
    assert!(mirrors_messages(store));
    assert_eq!(text(store, "older runner"), ["m4", "m2"]);
    assert!(text(store, "m2").is_empty());
    // Every delete path takes the row along.
    store.remove("chat", "m4").unwrap();
    assert!(mirrors_messages(store));
    let mut elsewhere = message("elsewhere", 5.0);
    elsewhere.chat_id = "gone".into();
    store.upsert(&elsewhere).unwrap();
    store.retain_chats(&["chat".into()]).unwrap();
    assert!(mirrors_messages(store));
    store.save_state_deleting_chats(&State::default(), &["chat".into()]).unwrap();
    store.upsert(&message("again", 6.0)).unwrap();
    store.clear().unwrap();
    assert!(mirrors_messages(store));
    assert!(rows(store).is_empty());
}

/// A backlog of a million messages must not make each pass sort or scan all of them.
#[test]
fn every_queue_is_read_in_order_from_its_own_index() {
    let scratch = scratch();
    let connection = scratch.0.connection.lock().unwrap();
    for (sql, index) in [
        (sql!("vectors/queue_text"), "message_search_embed_queue"),
        (sql!("vectors/queue_images"), "message_search_image_queue"),
        (sql!("vectors/queue_descriptions"), "message_search_describe_queue"),
        (sql!("search/text_candidates"), "message_search_order"),
        (sql!("search/recent_texts"), "message_search_order"),
        (sql!("vectors/requeue_picture"), "message_search_pictures"),
    ] {
        let plan: Vec<String> = connection.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap().query_map([32], |r| r.get(3)).unwrap().map(Result::unwrap).collect();
        assert!(plan.iter().any(|step| step.contains(index)), "{index}: {plan:?}");
        assert!(!plan.iter().any(|step| step.contains("TEMP B-TREE")), "{index} sorts: {plan:?}");
    }
    // Asked each time the Search pane lists models: never a scan of every message.
    let plan: Vec<String> =
        connection.prepare(&format!("EXPLAIN QUERY PLAN {}", sql!("search/has_pictures"))).unwrap().query_map([], |r| r.get(3)).unwrap().map(Result::unwrap).collect();
    assert!(plan.iter().any(|step| step.contains("message_search_pictures")), "{plan:?}");
}
