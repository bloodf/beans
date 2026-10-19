use beans::artifacts::{
    acceptance_fingerprint, pin_id, proposal_id, removed_chat_id, ArtifactAuthority,
    ArtifactEnvelope, ArtifactFailure, ArtifactOrigin, ArtifactSave,
};
use serde_json::json;

#[test]
fn artifact_parser_and_canonical_boundary_preserve_immutable_intent() {
    let empty_hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let revision_id = "00000000-0000-4000-8000-000000000001";
    let id = proposal_id("chat-fixture", "bot-fixture", "card-fixture").unwrap();
    let root = json!({
        "version": 1, "id": id, "revision_id": revision_id,
        "name": "note.md", "mime": "text/markdown", "size": 0,
        "content_hash": empty_hash, "parent_revision": null, "parent_content_hash": null,
        "file_id": format!("{revision_id}.file"), "chat_id": "chat-fixture",
        "bot_id": "bot-fixture", "runner_id": "runner-fixture", "card_id": "card-fixture",
        "attachment_id": null, "created_at": 1.0, "path": "notes/note.md"
    });
    let decode = |value: &serde_json::Value| ArtifactEnvelope::parse(&serde_json::to_vec(value).unwrap());
    let ArtifactEnvelope::Revision(revision) = decode(&root).unwrap() else { panic!("revision expected") };
    revision.verify_bytes(b"").unwrap();
    assert_eq!(revision.verify_bytes(b"changed"), Err(ArtifactFailure::Invalid));
    revision.validate_parent(None).unwrap();

    for field in ["parent_revision", "parent_content_hash", "bot_id", "card_id", "attachment_id", "path"] {
        let mut omitted = root.clone();
        omitted.as_object_mut().unwrap().remove(field);
        assert_eq!(decode(&omitted), Err(ArtifactFailure::Invalid), "missing {field}");
    }
    let mut extra = root.clone();
    extra["secret"] = json!("must-not-be-accepted");
    assert_eq!(decode(&extra), Err(ArtifactFailure::Invalid));
    let raw = serde_json::to_string(&root).unwrap();
    let duplicate = format!("{{\"version\":1,{}", &raw[1..]);
    assert_eq!(ArtifactEnvelope::parse(duplicate.as_bytes()), Err(ArtifactFailure::Invalid));
    for (field, value) in [
        ("version", json!(2)), ("size", json!(104_857_601_u64)),
        ("content_hash", json!(empty_hash.to_uppercase())), ("path", json!("../outside")),
        ("file_id", json!("other.file")), ("parent_revision", json!(revision_id)),
        ("op", json!("remove")),
    ] {
        let mut invalid = root.clone();
        invalid[field] = value;
        assert_eq!(decode(&invalid), Err(ArtifactFailure::Invalid), "invalid {field}");
    }
    assert!(matches!(decode(&json!({"version":1,"op":"remove","id":id,"chat_id":"chat-fixture"})).unwrap(), ArtifactEnvelope::Remove(_)));
    assert!(matches!(decode(&json!({"version":1,"op":"remove_chat","chat_id":"chat-fixture"})).unwrap(), ArtifactEnvelope::RemoveChat(_)));
    assert_eq!(decode(&json!({"version":1,"op":"other","chat_id":"chat-fixture"})), Err(ArtifactFailure::Invalid));
    assert_eq!(decode(&json!({"version":1,"op":"remove_chat","chat_id":"chat-fixture","id":id})), Err(ArtifactFailure::Invalid));
    assert_eq!(removed_chat_id("chat-fixture").unwrap(), "ach-bd0ce2c439c239db194ed22245dd3118c620c402df3ef8ca.removed");
    assert_ne!(proposal_id("ab", "c", "d").unwrap(), proposal_id("a", "bc", "d").unwrap());
    assert_ne!(pin_id("chat-fixture", "card-fixture").unwrap(), id);
    assert!(proposal_id("bad\0chat", "b", "c").is_err());

    let authority = ArtifactAuthority {
        account_id: "account-fixture".into(), incarnation: 1, requester: "device-fixture".into(),
        runner_id: "runner-fixture".into(), owner_epoch: "owner-fixture".into(), account_epoch: "epoch-fixture".into(),
    };
    let origin = ArtifactOrigin::Proposal { chat_id: "chat-fixture".into(), bot_id: "bot-fixture".into(), card_id: "card-fixture".into() };
    let commitment = acceptance_fingerprint(&authority, &origin, "request-fixture", &revision, None).unwrap();
    let mut retry = revision.clone();
    retry.revision_id = "00000000-0000-4000-8000-000000000002".into();
    retry.file_id = format!("{}.file", retry.revision_id);
    retry.created_at = 2.0;
    assert_eq!(acceptance_fingerprint(&authority, &origin, "request-fixture", &retry, None).unwrap(), commitment);
    retry.path = Some("notes/other.md".into());
    assert_ne!(acceptance_fingerprint(&authority, &origin, "request-fixture", &retry, None).unwrap(), commitment);
    let mut foreign = authority.clone();
    foreign.account_id = "another-account".into();
    assert_ne!(acceptance_fingerprint(&foreign, &origin, "request-fixture", &revision, None).unwrap(), commitment);

    let mut child = revision.clone();
    child.revision_id = "00000000-0000-4000-8000-000000000003".into();
    child.file_id = format!("{}.file", child.revision_id);
    child.parent_revision = Some(revision.revision_id.clone());
    child.parent_content_hash = Some(revision.content_hash.clone());
    assert_eq!(child.validate_parent(None), Err(ArtifactFailure::Incomplete));
    child.validate_parent(Some(&revision)).unwrap();
    child.chat_id = "foreign-chat".into();
    assert_eq!(child.validate_parent(Some(&revision)), Err(ArtifactFailure::Invalid));

    let save = json!({"id":id,"expected_revision":revision_id,"expected_hash":empty_hash,"content":"x".repeat(20_000),"path":null,"request_id":"request-fixture"});
    ArtifactSave::parse(&serde_json::to_vec(&save).unwrap()).unwrap();
    let mut too_many_chars = save.clone();
    too_many_chars["content"] = json!("x".repeat(20_001));
    assert_eq!(ArtifactSave::parse(&serde_json::to_vec(&too_many_chars).unwrap()), Err(ArtifactFailure::Invalid));
    let mut too_many_bytes = save.clone();
    too_many_bytes["content"] = json!("🌍".repeat(16_385));
    assert_eq!(ArtifactSave::parse(&serde_json::to_vec(&too_many_bytes).unwrap()), Err(ArtifactFailure::Invalid));
    let mut missing_path = save;
    missing_path.as_object_mut().unwrap().remove("path");
    assert_eq!(ArtifactSave::parse(&serde_json::to_vec(&missing_path).unwrap()), Err(ArtifactFailure::Invalid));
}
