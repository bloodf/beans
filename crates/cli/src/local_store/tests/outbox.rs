//! The relay outbox and orphan cleanup.

use super::*;

#[test]
fn queued_ciphertext_is_binary_and_survives_reopening() {
    let scratch = scratch();
    let ciphertext = vec![0, 255, 128, 13, 10, 34];
    let item = OutboxItem {
        id: "att-file".into(), kind: "file".into(), recipient: None,
        ciphertext: ciphertext.clone(), slot: None, group: Some("chat".into()),
    };
    scratch.0.queue_outbox(&item).unwrap();
    let connection = scratch.0.connection.lock().unwrap();
    let (kind, length): (String, usize) = connection.query_row(
        "SELECT typeof(ciphertext), length(ciphertext) FROM outbox WHERE id = 'att-file'",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(kind, "blob");
    assert_eq!(length, ciphertext.len());
    drop(connection);
    let reopened = LocalStore::open(&scratch.1.join("beans.sqlite3")).unwrap();
    let queued = reopened.first_outbox().unwrap().unwrap();
    assert_eq!(queued.ciphertext, ciphertext);
    assert_eq!(queued.group.as_deref(), Some("chat"));
}

#[test]
fn a_waiting_slot_is_replaced_without_moving_in_the_outbox() {
    let scratch = scratch();
    let item = |id: &str, slot: Option<&str>| OutboxItem {
        id: id.into(),
        kind: "chat".into(),
        recipient: None,
        ciphertext: id.as_bytes().to_vec(),
        slot: slot.map(|name| Slot {
            name: name.into(),
            keep_first: true,
        }),
        group: Some("chat".into()),
    };
    scratch
        .0
        .queue_outbox(&item("first", Some("message")))
        .unwrap();
    scratch.0.queue_outbox(&item("second", None)).unwrap();
    scratch
        .0
        .queue_outbox(&item("latest", Some("message")))
        .unwrap();

    let outbox = scratch.0.outbox().unwrap();
    assert_eq!(
        outbox
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["latest", "second"]
    );
    assert_eq!(
        scratch.0.first_outbox().unwrap().unwrap().ciphertext,
        b"latest"
    );
}

#[test]
fn metadata_prunes_orphaned_messages_and_grouped_uploads() {
    let scratch = scratch();
    let mut kept = message("kept", 1.0);
    kept.chat_id = "kept-chat".into();
    let mut orphan = message("orphan", 2.0);
    orphan.chat_id = "orphan-chat".into();
    scratch.0.upsert(&kept).unwrap();
    scratch.0.upsert(&orphan).unwrap();
    for (id, group) in [
        ("kept-upload", "kept-chat"),
        ("orphan-upload", "orphan-chat"),
    ] {
        scratch
            .0
            .queue_outbox(&OutboxItem {
                id: id.into(),
                kind: "chat".into(),
                recipient: None,
                ciphertext: Vec::new(),
                slot: None,
                group: Some(group.into()),
            })
            .unwrap();
    }

    scratch.0.retain_chats(&["kept-chat".into()]).unwrap();

    assert!(scratch.0.message("kept-chat", "kept").unwrap().is_some());
    assert!(scratch
        .0
        .message("orphan-chat", "orphan")
        .unwrap()
        .is_none());
    assert_eq!(
        scratch
            .0
            .outbox()
            .unwrap()
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["kept-upload"]
    );
}
