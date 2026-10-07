use super::*;
fn scope(bot: &str) -> MemoryScope {
    MemoryScope {
        namespace: namespace(&crate::keys::b64(&[9; 32]), bot).unwrap(),
        connection_id: "test".into(),
        connection_revision: Revision {
            counter: 1,
            device_id: "runner".into(),
        },
        deletion_epoch: 0,
    }
}
fn space() -> VectorSpace {
    VectorSpace::new("a".repeat(64), 3, VectorDistance::Cosine).unwrap()
}
fn doc(id: &str, text: &str) -> FrozenDocument {
    FrozenDocument {
        id: id.into(),
        request_id: format!("r-{id}"),
        text: text.into(),
        sources: vec![Source {
            chat_id: "synthetic".into(),
            message_id: id.into(),
            speaker: Speaker::User,
        }],
        content_hash: digest(text.as_bytes()),
    }
}
#[test]
fn scope_binds_account_bot_and_connection_revision() {
    let a = scope("a");
    bind(&a, &a).unwrap();
    assert!(bind(&a, &scope("b")).is_err());
    let mut foreign = a.clone();
    foreign.connection_revision.counter += 1;
    assert!(bind(&a, &foreign).is_err());
    foreign = a.clone();
    foreign.namespace = "';DELETE FROM documents;--".into();
    assert!(bind(&foreign, &foreign).is_err());
}
#[test]
fn vector_space_and_document_invariants_reject_other_model_and_corruption() {
    let s = space();
    assert!(s.validate_raw(&"b".repeat(64), &[1., 0., 0.]).is_err());
    assert!(s
        .validate_raw(&s.fingerprint, &[f32::INFINITY, 0., 0.])
        .is_err());
    assert!(s.validate_raw(&s.fingerprint, &[0., 0., 0.]).is_err());
    let mut d = doc("id", "evidence");
    document_valid(&d).unwrap();
    d.text = "changed".into();
    assert!(document_valid(&d).is_err());
}
#[test]
fn deletion_epoch_is_part_of_vector_scope() {
    let mut bound = scope("a");
    bound.deletion_epoch = 7;
    for epoch in [6, 8] {
        let mut changed = bound.clone();
        changed.deletion_epoch = epoch;
        assert_eq!(bind(&bound, &changed).unwrap_err().code, "memory_scope_mismatch");
    }
    bind(&bound, &bound).unwrap();
}

#[test]
fn curation_capability_is_exact_edit_only() {
    let capabilities = capabilities();
    let edit = BackendRequest::Advanced {
        feature: AdvancedFeature::MemoryEdit,
        action: "edit".into(),
        body: serde_json::json!({"document_id":"id","text":"curated"}),
    };
    assert!(capabilities.supports(&edit));
    let rejected = BackendRequest::Advanced {
        feature: AdvancedFeature::MemoryEdit,
        action: "delete".into(),
        body: serde_json::json!({"document_id":"id"}),
    };
    assert!(!capabilities.supports(&rejected));
}
#[test]
fn postgres_path_cannot_inject_another_authority_or_user() {
    let result = PgvectorConfig::new(
        "postgresql://expected.example/db@attacker.example/db",
        "memory",
        space(),
    );
    if let Ok(config) = result {
        assert_eq!(config.connection.get_hosts(), &[tokio_postgres::config::Host::Tcp("expected.example".into())]);
        assert!(config.connection.get_user().is_none());
        assert!(config.connection.get_password().is_none());
        let config = config.with_password(Some("synthetic-password")).unwrap();
        assert_eq!(config.connection.get_hosts(), &[tokio_postgres::config::Host::Tcp("expected.example".into())]);
    }
}
#[tokio::test]
#[ignore = "Requires BEANS_MEMORY_PGVECTOR_SANDBOX=1 and explicit disposable BEANS_MEMORY_PGVECTOR_SANDBOX_URL; never production"]
async fn disposable_postgres_preview_approval_scoped_crud() {
    assert_eq!(
        std::env::var("BEANS_MEMORY_PGVECTOR_SANDBOX").as_deref(),
        Ok("1"),
        "explicit sandbox authorization required"
    );
    let url = std::env::var("BEANS_MEMORY_PGVECTOR_SANDBOX_URL")
        .expect("explicit disposable database URL required");
    let schema = format!("memory_fixture_{}", uuid::Uuid::new_v4().simple());
    let mut config = PgvectorConfig::new(
        &url.replace("sslmode=disable", "sslmode=require"),
        &schema,
        space(),
    )
    .unwrap();
    assert!(config.connection.get_hosts().iter().all(|h|matches!(h,tokio_postgres::config::Host::Tcp(host) if host=="localhost"||host=="127.0.0.1"||host=="::1")),"fixture must be loopback disposable database");
    config.connection.ssl_mode(SslMode::Disable);
    let a = scope("a");
    let backend = PgvectorBackend::sandbox(config, a.clone()).await.unwrap();
    // Holding the actual client makes any attempted database access wait. Epoch rejection
    // must complete without acquiring it, before SQL/schema inspection or embedding work.
    {
        let _client = backend.client.lock().await;
        let mut changed = a.clone();
        changed.deletion_epoch += 1;
        let requests = vec![
            BackendRequest::Health,
            BackendRequest::Retain { document: doc("id", "rejected") },
            BackendRequest::Recall { query: "evidence".into(), budget: RecallBudget::default() },
            BackendRequest::Inspect { document_id: "id".into() },
            BackendRequest::DeleteDocument { document_id: "id".into(), request_id: "delete".into() },
            BackendRequest::Clear { request_id: "clear".into() },
            BackendRequest::Advanced { feature: AdvancedFeature::MemoryEdit, action: "edit".into(), body: serde_json::json!({"document_id":"id","text":"rejected"}) },
        ];
        for request in requests {
            let result = tokio::time::timeout(Duration::from_millis(100), backend.execute(&changed, request, CancellationToken::new())).await.expect("epoch check must precede client access");
            assert_eq!(result.unwrap_err().code, "memory_scope_mismatch");
        }
        let result = tokio::time::timeout(Duration::from_millis(100), backend.retain_vector(&changed, doc("id","rejected"), &space().fingerprint, vec![1.,0.,0.], CancellationToken::new())).await.expect("epoch check must precede client access");
        assert_eq!(result.unwrap_err().code, "memory_scope_mismatch");
        let result = tokio::time::timeout(Duration::from_millis(100), backend.recall_vector(&changed, &space().fingerprint, vec![1.,0.,0.], RecallBudget::default(), CancellationToken::new())).await.expect("epoch check must precede client access");
        assert_eq!(result.unwrap_err().code, "memory_scope_mismatch");
    }
    assert!(backend.readiness(CancellationToken::new()).await.is_err());
    assert_eq!(
        backend
            .initialize_apply("unapproved", CancellationToken::new())
            .await
            .unwrap_err()
            .code,
        "schema_approval_required"
    );
    let preview = backend
        .initialize_preview(CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(preview.schema, schema);
    assert!(!preview.sql.contains("DROP"));
    assert!(backend
        .initialize_apply("wrong-token", CancellationToken::new())
        .await
        .is_err());
    assert!(
        backend
            .initialize_apply(&preview.approval_token, CancellationToken::new())
            .await
            .is_err(),
        "wrong attempt consumes token"
    );
    let preview = backend
        .initialize_preview(CancellationToken::new())
        .await
        .unwrap();
    backend
        .initialize_apply(&preview.approval_token, CancellationToken::new())
        .await
        .unwrap();
    assert!(backend
        .initialize_apply(&preview.approval_token, CancellationToken::new())
        .await
        .is_err());
    let id = "x'; DELETE FROM public.documents; --";
    backend
        .retain_vector(
            &a,
            doc(id, "own"),
            &"a".repeat(64),
            vec![1., 0., 0.],
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(backend
        .retain_vector(
            &scope("b"),
            doc(id, "foreign"),
            &"a".repeat(64),
            vec![1., 0., 0.],
            CancellationToken::new()
        )
        .await
        .is_err());
    backend
        .retain_vector(
            &a,
            doc(id, "updated"),
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
    assert_eq!(found.evidence[0].text, "updated");
    assert_eq!(
        backend
            .inspect(&a, id, CancellationToken::new())
            .await
            .unwrap()
            .document
            .unwrap()
            .sources[0]
            .message_id,
        id
    );
    backend
        .delete(&a, Some(id), CancellationToken::new())
        .await
        .unwrap();
    assert!(backend
        .inspect(&a, id, CancellationToken::new())
        .await
        .unwrap()
        .document
        .is_none());
    backend
        .client
        .lock()
        .await
        .batch_execute(&format!("DROP SCHEMA \"{schema}\" CASCADE"))
        .await
        .unwrap();
}

fn catalog_fixture(config: &PgvectorConfig) -> Vec<CatalogConstraint> {
    let constraint = |table: &str, definition: String, kind: &str, columns: &[&str]| CatalogConstraint {
        table: table.into(), definition, kind: kind.into(), validated: true,
        columns: columns.iter().map(|column| (*column).into()).collect(),
        referenced_schema: None, referenced_table: None, referenced_columns: Vec::new(), delete_action: "a".into(),
    };
    let mut catalog = vec![
        constraint("documents", "PRIMARY KEY (namespace, document_id)".into(), "p", &["namespace","document_id"]),
        constraint("chunks", "PRIMARY KEY (namespace, document_id, chunk_id)".into(), "p", &["namespace","document_id","chunk_id"]),
        constraint("chunks", "CHECK ((chunk_id = 0))".into(), "c", &["chunk_id"]),
        constraint("chunks", format!("CHECK ((fingerprint = '{}'::text))", config.space.fingerprint), "c", &["fingerprint"]),
        constraint("chunks", format!("CHECK ((index_generation = {}))", config.space.generation), "c", &["index_generation"]),
    ];
    let mut foreign = constraint("chunks", format!("FOREIGN KEY (namespace, document_id) REFERENCES {}.documents(namespace, document_id) ON DELETE CASCADE", config.schema), "f", &["namespace","document_id"]);
    foreign.referenced_schema = Some(config.schema.clone());
    foreign.referenced_table = Some("documents".into());
    foreign.referenced_columns = vec!["namespace".into(),"document_id".into()];
    foreign.delete_action = "c".into();
    catalog.push(foreign);
    catalog
}

#[test]
fn readiness_rejects_missing_single_chunk_constraint() {
    let config = PgvectorConfig::new("postgresql://localhost/sandbox?sslmode=require", "memory", space()).unwrap();
    let mut catalog = catalog_fixture(&config);
    config.validate_constraints(&catalog).unwrap();
    catalog.retain(|constraint| constraint.definition != "CHECK ((chunk_id = 0))");
    assert_eq!(config.validate_constraints(&catalog).unwrap_err().code, "memory_schema_not_ready");
}

#[test]
fn readiness_rejects_wrong_or_unvalidated_cascade_parent() {
    let config = PgvectorConfig::new("postgresql://localhost/sandbox?sslmode=require", "memory", space()).unwrap();
    let original = catalog_fixture(&config);
    config.validate_constraints(&original).unwrap();
    for defect in ["parent", "schema", "keys", "cascade", "validated"] {
        let mut catalog = original.clone();
        let foreign = catalog.iter_mut().find(|constraint| constraint.kind == "f").unwrap();
        match defect {
            "parent" => {
                foreign.referenced_table = Some("shadow_documents".into());
                foreign.definition = "FOREIGN KEY (namespace, document_id) REFERENCES memory.shadow_documents(namespace, document_id) ON DELETE CASCADE".into();
            },
            "schema" => foreign.referenced_schema = Some("other".into()),
            "keys" => foreign.referenced_columns.reverse(),
            "cascade" => foreign.delete_action = "a".into(),
            "validated" => foreign.validated = false,
            _ => unreachable!(),
        }
        assert_eq!(config.validate_constraints(&catalog).unwrap_err().code, "memory_schema_not_ready", "{defect}");
    }
}
