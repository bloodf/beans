#![cfg(feature = "runner")]
use beans::memory_service::{
    backends::{
        lance::{LanceBackend, LanceBinding},
        pgvector::{PgvectorConfig, VectorDistance, VectorSpace},
    },
    types::*,
};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

fn scope(bot: &str) -> MemoryScope {
    MemoryScope {
        namespace: namespace(&beans::keys::b64(&[42; 32]), bot).unwrap(),
        connection_id: "vectors".into(),
        connection_revision: Revision {
            counter: 1,
            device_id: "runner".into(),
        },
        deletion_epoch: 0,
    }
}
fn doc(id: &str, text: &str) -> FrozenDocument {
    FrozenDocument {
        id: id.into(),
        request_id: format!("request-{id}"),
        text: text.into(),
        sources: vec![Source {
            chat_id: "synthetic".into(),
            message_id: id.into(),
            speaker: Speaker::User,
        }],
        content_hash: digest(text.as_bytes()),
    }
}
fn space() -> VectorSpace {
    VectorSpace::new("a".repeat(64), 3, VectorDistance::Cosine).unwrap()
}

#[test]
fn pgvector_config_rejects_sql_selector_and_plaintext_tls() {
    assert!(PgvectorConfig::new(
        "postgresql://localhost/sandbox?sslmode=disable",
        "memory",
        space()
    )
    .is_err());
    assert!(PgvectorConfig::new(
        "postgresql://localhost/sandbox?sslmode=require",
        "memory; DROP SCHEMA public",
        space()
    )
    .is_err());
    let config = PgvectorConfig::new(
        "postgresql://localhost/sandbox?sslmode=require",
        "memory",
        space(),
    )
    .unwrap();
    assert!(!format!("{config:?}").contains("postgresql://"));
}

#[test]
fn vectors_reject_same_dimension_other_model_and_nonfinite() {
    let s = space();
    assert!(s.validate_raw(&"b".repeat(64), &[1., 0., 0.]).is_err());
    assert!(s.validate_raw(&"a".repeat(64), &[1., 0.]).is_err());
    assert!(s
        .validate_raw(&"a".repeat(64), &[f32::NAN, 0., 1.])
        .is_err());
    assert!(s.validate_raw(&"a".repeat(64), &[0., 0., 0.]).is_err());
    s.validate_raw(&"a".repeat(64), &[1., 0., 0.]).unwrap();
}

#[tokio::test]
async fn lance_real_temp_scoped_upsert_search_provenance_edit_delete_transfer() {
    let directory = tempfile::tempdir().unwrap();
    let a = scope("a");
    let b = scope("b");
    let backend = LanceBackend::open(
        LanceBinding::Local {
            directory: directory.path().join("db"),
            runner_id: "runner".into(),
        },
        a.clone(),
        space(),
        None,
        true,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    backend
        .retain_vector(
            &a,
            doc("same", "own evidence"),
            &"a".repeat(64),
            vec![1., 0., 0.],
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(backend
        .retain_vector(
            &b,
            doc("same", "foreign evidence"),
            &"a".repeat(64),
            vec![1., 0., 0.],
            CancellationToken::new()
        )
        .await
        .is_err());
    let sibling = LanceBackend::open(
        LanceBinding::Local {
            directory: directory.path().join("db"),
            runner_id: "runner".into(),
        },
        b.clone(),
        space(),
        None,
        false,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    sibling
        .retain_vector(
            &b,
            doc("same", "foreign evidence"),
            &"a".repeat(64),
            vec![1., 0., 0.],
            CancellationToken::new(),
        )
        .await
        .unwrap();
    backend
        .retain_vector(
            &a,
            doc("same", "updated own evidence"),
            &"a".repeat(64),
            vec![1., 0., 0.],
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let found = backend
        .recall_vector(
            &a,
            &"a".repeat(64),
            vec![1., 0., 0.],
            RecallBudget::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(found.evidence.len(), 1);
    assert_eq!(found.evidence[0].text, "updated own evidence");
    let inspected = backend
        .execute(
            &a,
            BackendRequest::Inspect {
                document_id: "same".into(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .document
        .unwrap();
    assert_eq!(inspected.sources[0].message_id, "same");
    assert_eq!(inspected.content_hash, digest(b"updated own evidence"));
    let export = backend.export(&a, CancellationToken::new()).await.unwrap();
    let moved_directory = tempfile::tempdir().unwrap();
    let moved = LanceBackend::open(
        LanceBinding::Local {
            directory: moved_directory.path().join("db"),
            runner_id: "other-runner".into(),
        },
        a.clone(),
        space(),
        None,
        true,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    moved
        .import(&a, export, true, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        moved
            .execute(
                &a,
                BackendRequest::Inspect {
                    document_id: "same".into()
                },
                CancellationToken::new()
            )
            .await
            .unwrap()
            .document
            .unwrap(),
        inspected
    );
    backend
        .execute(
            &a,
            BackendRequest::DeleteDocument {
                document_id: "same".into(),
                request_id: "delete".into(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(backend
        .execute(
            &a,
            BackendRequest::Inspect {
                document_id: "same".into()
            },
            CancellationToken::new()
        )
        .await
        .unwrap()
        .document
        .is_none());
    assert_eq!(
        sibling
            .execute(
                &b,
                BackendRequest::Inspect {
                    document_id: "same".into()
                },
                CancellationToken::new()
            )
            .await
            .unwrap()
            .document
            .unwrap()
            .text,
        "foreign evidence"
    );
    let unavailable = LanceBackend::open(
        LanceBinding::Local {
            directory: directory.path().join("missing"),
            runner_id: "other-runner".into(),
        },
        a.clone(),
        space(),
        None,
        false,
        CancellationToken::new(),
    )
    .await;
    assert!(unavailable.is_err());
    let wrong_model = VectorSpace::new("b".repeat(64), 3, VectorDistance::Cosine).unwrap();
    assert!(LanceBackend::open(
        LanceBinding::Local {
            directory: directory.path().join("db"),
            runner_id: "runner".into()
        },
        a,
        wrong_model,
        None,
        false,
        CancellationToken::new()
    )
    .await
    .is_err());
    drop(Arc::new(backend));
}
