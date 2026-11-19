//! The transcript: order, pages, context, and the rows turns read.

use super::*;

#[test]
fn older_pages_land_before_what_is_here() {
    let scratch = scratch();
    let store = &scratch.0;
    let said = |id: &str| message(id, 0.0);
    for id in ["m5", "m6"] {
        store.upsert(&said(id)).unwrap();
    }
    // An edit of an old message came through the log and went to the end.
    store.upsert(&said("m3")).unwrap();
    assert_eq!(store.history_before("chat").unwrap(), None);
    store.set_history_before("chat", Some(40)).unwrap();
    assert_eq!(store.history_before("chat").unwrap(), Some(40));

    store.insert_older(&[said("m4"), said("m3")]).unwrap();
    store.insert_older(&[said("m2"), said("m1")]).unwrap();
    store.set_history_before("chat", None).unwrap();
    let ids = |messages: Vec<Message>| messages.into_iter().map(|m| m.id).collect::<Vec<_>>();
    assert_eq!(ids(store.all("chat").unwrap()), ["m1", "m2", "m3", "m4", "m5", "m6"]);
    let (page, more) = store.page("chat", Some("m5"), 2).unwrap();
    assert_eq!((ids(page), more), (vec!["m3".to_string(), "m4".into()], true));
    assert_eq!(store.history_before("chat").unwrap(), None);
}

#[test]
fn upserts_keep_position_and_pages_are_stable() {
    let scratch = scratch();
    for i in 0..5 {
        scratch
            .0
            .upsert(&message(&format!("m{i}"), i as f64))
            .unwrap();
    }
    let (page, more) = scratch.0.page("chat", None, 2).unwrap();
    assert_eq!(
        page.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["m3", "m4"]
    );
    assert!(more);
    let mut changed = message("m3", 3.0);
    changed.state = MessageState::Streaming;
    scratch.0.upsert(&changed).unwrap();
    let (page, _) = scratch
        .0
        .page("chat", Some("m4"), SNAPSHOT_MESSAGES)
        .unwrap();
    assert_eq!(
        page.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["m0", "m1", "m2", "m3"]
    );
}

#[test]
fn context_orders_promoted_messages_and_honors_a_compaction_cursor() {
    let scratch = scratch();
    scratch.0.upsert(&message("first", 1.0)).unwrap();
    let mut promoted = message("steer", 2.0);
    promoted.promoted_at = Some(4.0);
    scratch.0.upsert(&promoted).unwrap();
    scratch.0.upsert(&message("settled", 3.0)).unwrap();
    let (messages, found) = scratch.0.context("chat", None, Some(10)).unwrap();
    assert!(!found);
    assert_eq!(
        messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["first", "settled", "steer"]
    );
    let (messages, found) = scratch.0.context("chat", Some("settled"), Some(1)).unwrap();
    assert!(found);
    assert_eq!(
        messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["steer"]
    );
}

#[test]
fn indexed_activity_queries_do_not_load_tool_payloads() {
    let scratch = scratch();
    let mut user = message("user", 1.0);
    user.body = Body::text("hello");
    let mut own = message("own", 2.0);
    own.author = Author::Bot {
        bot_id: "chef".into(),
    };
    own.body = Body::text("done");
    let mut other = message("other", 3.0);
    other.author = Author::Bot {
        bot_id: "scout".into(),
    };
    other.body = Body::text("found it");
    let mut handoff = message("handoff", 4.0);
    handoff.author = Author::Bot {
        bot_id: "scout".into(),
    };
    handoff.body = Body::Handoff {
        from: "scout".into(),
        to: "chef".into(),
        reason: "take over".into(),
    };
    let mut empty = message("empty", 5.0);
    empty.body = Body::text("  ");
    for row in [user, own, other, handoff, empty] {
        scratch.0.upsert(&row).unwrap();
    }

    assert_eq!(
        scratch.0.last_user_text("chat", "user").unwrap().as_deref(),
        Some("hello")
    );
    assert_eq!(scratch.0.last_user_text("chat", "handoff").unwrap(), None);
    assert_eq!(
        scratch.0.last_bot_text("chat", "chef").unwrap().unwrap().id,
        "own"
    );
    assert_eq!(scratch.0.heard_count("chat", "chef").unwrap(), 4);
    assert_eq!(
        scratch.0.new_count_since_bot_spoke("chat", "chef").unwrap(),
        3
    );
    assert_eq!(
        scratch
            .0
            .text_messages("chat", Some(2), Some(3))
            .unwrap()
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["own", "other"]
    );
}

#[test]
fn a_turn_reads_the_request_that_started_it() {
    let scratch = scratch();
    let store = &scratch.0;
    let add = |id: &str, author: Author, body: Body| {
        let mut row = message(id, 0.0);
        row.author = author;
        row.body = body;
        store.upsert(&row).unwrap();
    };
    let devops = || Author::Bot {
        bot_id: "devops".into(),
    };
    add("stop", Author::You, Body::text("actually stop that"));
    add("stopped", devops(), Body::text("Stopped."));
    add(
        "handoff",
        Author::Bot {
            bot_id: "chef".into(),
        },
        Body::Handoff {
            from: "chef".into(),
            to: "devops".into(),
            reason: "You own Railway monitoring".into(),
        },
    );
    let card = serde_json::from_value(serde_json::json!({
        "kind": "tool", "name": "bash", "summary": "", "detail": "", "is_running": false
    }))
    .unwrap();
    add("card", devops(), card);
    add(
        "notice",
        Author::System,
        Body::Notice {
            text: "DevOps cannot run yet".into(),
            routine_id: None,
        },
    );
    add(
        "routine",
        Author::System,
        Body::Notice {
            text: "Routine · Railway memory watch".into(),
            routine_id: Some("rt-1".into()),
        },
    );

    let request = |id: &str| store.request_at("chat", id).unwrap().map(|message| message.id);
    assert_eq!(request("stop").as_deref(), Some("stop"));
    assert_eq!(request("stopped").as_deref(), Some("stop"));
    assert_eq!(request("handoff").as_deref(), Some("handoff"));
    // A command's end opens a turn on its card, after the request it served.
    assert_eq!(request("card").as_deref(), Some("handoff"));
    assert_eq!(request("notice").as_deref(), Some("handoff"));
    assert_eq!(request("routine").as_deref(), Some("routine"));
    assert_eq!(request("gone"), None);

    // What a turn did since its request, newest last, and how much a limit left out.
    let after = |id: &str, limit: usize| {
        let (messages, left_out) = store.newest_after("chat", id, limit).unwrap();
        (messages.into_iter().map(|message| message.id).collect::<Vec<_>>(), left_out)
    };
    assert_eq!(after("handoff", 10), (vec!["card".to_string(), "notice".into(), "routine".into()], 0));
    assert_eq!(after("handoff", 2), (vec!["notice".to_string(), "routine".into()], 1));
    assert_eq!(after("gone", 2), (Vec::new(), 0));

    // The user's stop came before the handoff, so the turn it started does not hear it.
    assert_eq!(store.last_user_text("chat", "handoff").unwrap(), None);
    assert_eq!(
        store.last_user_text("chat", "stop").unwrap().as_deref(),
        Some("actually stop that")
    );
    add("steer", Author::You, Body::text("leave Postgres alone"));
    assert_eq!(
        store.last_user_text("chat", "handoff").unwrap().as_deref(),
        Some("leave Postgres alone")
    );
    // Files sent with no words still start the turn, and nothing earlier speaks for them.
    add("files", Author::You, Body::text(""));
    assert_eq!(request("files").as_deref(), Some("files"));
    assert_eq!(store.last_user_text("chat", "files").unwrap(), None);
}
