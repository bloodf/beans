use super::super::http::test_server::{serve, Reply};
use super::*;
use serde_json::json;

fn scope() -> MemoryScope {
    MemoryScope {
        namespace: "a".repeat(64),
        connection_id: "fixture".into(),
        connection_revision: Revision::default(),
        deletion_epoch: 0,
    }
}
fn connection(endpoint: String) -> Connection {
    Connection {
        backend: BackendKind::Hindsight,
        name: "fixture".into(),
        endpoint: Some(endpoint),
        secret: Some("fixture-only-secret".into()),
        embedding_profile: None,
        options: Some(BackendOptions::Hindsight {}),
        allow_insecure_http: true,
        extra: Default::default(),
    }
}
fn handshake() -> Vec<Reply> {
    vec![
        Reply::json(
            json!({"api_version":"0.10.2","features":{"worker":true,"observations":true,"bank_config_api":true,"store_document_text":true}}),
        ),
        Reply::json(serde_json::from_str(include_str!("hindsight_schema.json")).unwrap()),
    ]
}
async fn adapter(replies: Vec<Reply>) -> (Hindsight, super::super::http::test_server::Server) {
    let mut all = handshake();
    all.extend(replies);
    let server = serve(all).await;
    let backend = Hindsight::connect(
        &connection(server.url.clone()),
        &scope(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    (backend, server)
}

#[tokio::test]
async fn async_acceptance_is_not_storage_and_preserves_idempotency() {
    let op = operation_uuid(&scope().namespace, &"b".repeat(64));
    let (backend, server) = adapter(vec![Reply::json(json!({"success":true,"bank_id":"a".repeat(64),"items_count":1,"async":true,"operation_id":op}))]).await;
    let document = FrozenDocument {
        id: "doc-1".into(),
        request_id: "b".repeat(64),
        text: "Synthetic fixture prefers tea".into(),
        content_hash: "c".repeat(64),
        sources: vec![],
    };
    let result = backend
        .execute(
            &scope(),
            BackendRequest::Retain { document },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.operation.unwrap().state, OperationState::Submitted);
    let requests = server.finish().await;
    let body = &requests[2].body;
    assert_eq!(body["async"], true);
    assert_eq!(body["items"][0]["document_id"], "doc-1");
    assert!(uuid::Uuid::parse_str(body["operation_id"].as_str().unwrap()).is_ok());
    assert!(backend.capabilities().idempotent_retain);
}

#[tokio::test]
async fn operation_completion_is_explicit_and_failed_is_not_stored() {
    let (backend, server) = adapter(vec![
        Reply::json(json!({"operation_id":"op","status":"processing"})),
        Reply::json(json!({"operation_id":"op","status":"completed"})),
        Reply::json(
            json!({"operation_id":"op","status":"failed","error_message":"fixture-only-secret"}),
        ),
    ])
    .await;
    for expected in [
        OperationState::Processing,
        OperationState::Completed,
        OperationState::Failed,
    ] {
        let response = backend
            .execute(
                &scope(),
                BackendRequest::OperationStatus {
                    operation_id: "op".into(),
                },
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(response.operation.unwrap().state, expected);
        assert!(!response.data.to_string().contains("fixture-only-secret"));
    }
    server.finish().await;
}

#[tokio::test]
async fn bank_profile_uses_config_and_cancel_uses_operation_not_record_delete() {
    let (backend, server) = adapter(vec![
        Reply::json(
            json!({"bank_id":"a".repeat(64),"config":{"reflect_mission":"fixture"},"overrides":{}}),
        ),
        Reply::json(json!({"success":true,"message":"cancelled","operation_id":"op"})),
    ])
    .await;
    backend
        .execute(
            &scope(),
            BackendRequest::Advanced {
                feature: AdvancedFeature::BankProfile,
                action: "get".into(),
                body: json!({}),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let response = backend
        .execute(
            &scope(),
            BackendRequest::CancelOperation {
                operation_id: "op".into(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_ne!(response.operation.unwrap().state, OperationState::Completed);
    let requests = server.finish().await;
    assert_eq!(
        requests[2].path,
        format!("/v1/default/banks/{}/config", "a".repeat(64))
    );
    assert_eq!(requests[3].method, "DELETE");
    assert_eq!(
        requests[3].path,
        format!("/v1/default/banks/{}/operations/op", "a".repeat(64))
    );
}

#[tokio::test]
async fn hostile_scope_and_advanced_selectors_fail_before_network() {
    let (backend, server) = adapter(vec![]).await;
    let mut other = scope();
    other.namespace = "b".repeat(64);
    assert_eq!(
        backend
            .execute(&other, BackendRequest::Health, CancellationToken::new())
            .await
            .unwrap_err()
            .code,
        "scope_mismatch"
    );
    for body in [
        json!({"bank_id":"other"}),
        json!({"updates":{"api_key":"secret"}}),
        json!({"updates":{"retain_strategies":{"nested":{"base_url":"bad"}}}}),
    ] {
        assert!(backend
            .execute(
                &scope(),
                BackendRequest::Advanced {
                    feature: AdvancedFeature::BankConfig,
                    action: "update".into(),
                    body
                },
                CancellationToken::new()
            )
            .await
            .is_err());
    }
    assert_eq!(server.finish().await.len(), 2);
}

#[tokio::test]
async fn capability_requires_actual_route_and_flags_not_version_guess() {
    let mut schema: Value = serde_json::from_str(include_str!("hindsight_schema.json")).unwrap();
    schema["paths"]
        .as_object_mut()
        .unwrap()
        .remove("/v1/default/banks/{bank_id}/reflect");
    let server = serve(vec![Reply::json(json!({"api_version":"0.10.2","features":{"worker":true,"observations":false,"bank_config_api":false,"store_document_text":false}})), Reply::json(schema)]).await;
    let backend = Hindsight::connect(
        &connection(server.url.clone()),
        &scope(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(!backend.capabilities().reflect);
    assert!(!backend
        .capabilities()
        .advanced
        .contains(&AdvancedFeature::Observations));
    assert!(backend
        .execute(
            &scope(),
            BackendRequest::Advanced {
                feature: AdvancedFeature::BankConfig,
                action: "update".into(),
                body: json!({"updates":{"reflect_mission":"fixture"}})
            },
            CancellationToken::new()
        )
        .await
        .is_err());
    server.finish().await;
}

#[tokio::test]
async fn recall_checks_result_bank_and_bounds_evidence() {
    let (backend, server) = adapter(vec![Reply::json(json!({"results":[{"id":"fact-1","text":"Synthetic tea preference","document_id":"doc-1"}]})),
        Reply::json(json!({"bank_id":"other","results":[{"id":"foreign","text":"foreign"}]}))]).await;
    let first = backend
        .execute(
            &scope(),
            BackendRequest::Recall {
                query: "tea".into(),
                budget: RecallBudget::default(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(first.evidence[0].document_id.as_deref(), Some("doc-1"));
    assert_eq!(first.evidence[0].text, "Synthetic tea preference");
    assert!(backend
        .execute(
            &scope(),
            BackendRequest::Recall {
                query: "tea".into(),
                budget: RecallBudget::default()
            },
            CancellationToken::new()
        )
        .await
        .is_err());
    server.finish().await;
}

#[tokio::test]
async fn reflection_is_native_reflect_with_evidence_not_relabelled_recall() {
    let (backend, server) = adapter(vec![Reply::json(json!({"text":"Synthetic conclusion","based_on":{"memories":[{"id":"fact","text":"Synthetic evidence"}]}}))]).await;
    let result = backend
        .execute(
            &scope(),
            BackendRequest::Reflect {
                query: "Explain fixture".into(),
                budget: RecallBudget::default(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.data["text"], "Synthetic conclusion");
    assert_eq!(result.evidence[0].text, "Synthetic evidence");
    let requests = server.finish().await;
    assert!(requests[2].path.ends_with("/reflect"));
}

#[tokio::test]
async fn mental_model_create_and_refresh_are_pending_without_automatic_schedules() {
    let (backend, server) = adapter(vec![
        Reply::json(json!({"mental_model_id":"model","operation_id":"create-op"})),
        Reply::json(json!({"operation_id":"refresh-op","status":"queued"})),
        Reply::json(
            json!({"history":[{"status":"completed","content":"Synthetic model history"}]}),
        ),
    ])
    .await;
    let created=backend.execute(&scope(),BackendRequest::Advanced {feature:AdvancedFeature::MentalModels,action:"create".into(),
        body:json!({"name":"Synthetic model","source_query":"What does the fixture prefer?"})},CancellationToken::new()).await.unwrap();
    assert_eq!(created.operation.unwrap().state, OperationState::Submitted);
    let refreshed = backend
        .execute(
            &scope(),
            BackendRequest::Advanced {
                feature: AdvancedFeature::MentalModels,
                action: "refresh".into(),
                body: json!({"id":"model"}),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        refreshed.operation.unwrap().state,
        OperationState::Submitted
    );
    let history = backend
        .execute(
            &scope(),
            BackendRequest::Advanced {
                feature: AdvancedFeature::MentalModelHistory,
                action: "list".into(),
                body: json!({"id":"model"}),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        history.data["history"][0]["content"],
        "Synthetic model history"
    );
    for trigger in [
        json!({"refresh_after_consolidation":true}),
        json!({"refresh_cron":"0 * * * *"}),
    ] {
        assert!(backend
            .execute(
                &scope(),
                BackendRequest::Advanced {
                    feature: AdvancedFeature::MentalModels,
                    action: "update".into(),
                    body: json!({"id":"model","trigger":trigger})
                },
                CancellationToken::new()
            )
            .await
            .is_err());
    }
    let seen = server.finish().await;
    assert_eq!(
        seen[2].body["trigger"]["refresh_after_consolidation"],
        false
    );
    assert_eq!(seen[2].body["trigger"]["refresh_cron"], Value::Null);
    assert!(seen[2].body["trigger"].get("refresh_cron").is_some());
}

#[tokio::test]
async fn invalidate_and_restore_have_distinct_official_states() {
    let (backend, server) = adapter(vec![
        Reply::json(json!({"id":"fact","state":"invalidated"})),
        Reply::json(json!({"id":"fact","state":"valid"})),
    ])
    .await;
    for (feature, state) in [
        (AdvancedFeature::MemoryInvalidate, "invalidated"),
        (AdvancedFeature::MemoryRestore, "valid"),
    ] {
        let result = backend
            .execute(
                &scope(),
                BackendRequest::Advanced {
                    feature,
                    action: "update".into(),
                    body: json!({"id":"fact","reason":"Synthetic correction"}),
                },
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(result.data["state"], state);
    }
    let seen = server.finish().await;
    assert_eq!(seen[2].body["state"], "invalidated");
    assert_eq!(seen[3].body["state"], "valid");
}

#[tokio::test]
async fn document_inspection_and_delete_refuse_mismatched_provenance() {
    let (backend, server) = adapter(vec![
        Reply::json(
            json!({"id":"other","bank_id":"a".repeat(64),"original_text":"Synthetic text"}),
        ),
        Reply::json(json!({"success":true,"document_id":"other","memory_units_deleted":1})),
    ])
    .await;
    assert_eq!(
        backend
            .execute(
                &scope(),
                BackendRequest::Inspect {
                    document_id: "doc".into()
                },
                CancellationToken::new()
            )
            .await
            .unwrap_err()
            .code,
        "document_mismatch"
    );
    assert_eq!(
        backend
            .execute(
                &scope(),
                BackendRequest::DeleteDocument {
                    document_id: "doc".into(),
                    request_id: "delete".into()
                },
                CancellationToken::new()
            )
            .await
            .unwrap_err()
            .code,
        "document_mismatch"
    );
    server.finish().await;
}

#[tokio::test]
async fn connection_revision_and_deletion_epoch_are_bound_not_ignored() {
    let (backend, server) = adapter(vec![]).await;
    let mut revision = scope();
    revision.connection_revision.counter += 1;
    let mut epoch = scope();
    epoch.deletion_epoch += 1;
    for scope in [revision, epoch] {
        assert_eq!(
            backend
                .execute(&scope, BackendRequest::Health, CancellationToken::new())
                .await
                .unwrap_err()
                .code,
            "scope_mismatch"
        );
    }
    assert_eq!(server.finish().await.len(), 2);
}

#[tokio::test]
async fn clear_deletes_the_complete_bank_not_only_extracted_memory_units() {
    let (backend, server) = adapter(vec![Reply::json(json!({"success":true}))]).await;
    backend
        .execute(
            &scope(),
            BackendRequest::Clear {
                request_id: "confirmed-clear".into(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let requests = server.finish().await;
    // The memories DELETE preserves bank profile; bank DELETE includes source documents.
    assert_eq!(requests[2].method, "DELETE");
    assert_eq!(
        requests[2].path,
        format!("/v1/default/banks/{}", scope().namespace)
    );
}

#[tokio::test]
async fn advanced_actions_require_exact_verbs_and_runtime_write_permissions() {
    for (config_writes, patch, reset, worker, status) in [
        (false, true, true, true, true),
        (true, false, true, true, true),
        (true, true, false, true, true),
        (true, true, true, false, true),
        (true, true, true, true, false),
    ] {
        let mut schema: Value =
            serde_json::from_str(include_str!("hindsight_schema.json")).unwrap();
        let paths = schema["paths"].as_object_mut().unwrap();
        let config = paths.get_mut(&format!("{BANK}/config")).unwrap();
        if !patch {
            config.as_object_mut().unwrap().remove("patch");
        }
        if !reset {
            config["delete"]["deprecated"] = json!(true);
        }
        if !status {
            paths
                .get_mut(&format!("{BANK}/operations/{{operation_id}}"))
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove("get");
        }
        paths
            .get_mut(&format!("{BANK}/directives/{{directive_id}}"))
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("patch");
        paths
            .get_mut(&format!("{BANK}/memories/{{memory_id}}/history"))
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("get");
        let server = serve(vec![
            Reply::json(json!({"api_version":"0.10.2","features":{
                "bank_config_api":config_writes,"worker":worker,"observations":true
            }})),
            Reply::json(schema),
            Reply::json(json!({"bank_id":scope().namespace,"config":{}})),
            Reply::json(json!({"bank_id":scope().namespace,"config":{}})),
        ])
        .await;
        let backend = Hindsight::connect(
            &connection(server.url.clone()),
            &scope(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        let capabilities = backend.capabilities();
        for feature in [AdvancedFeature::BankConfig, AdvancedFeature::BankProfile] {
            for (action, supported) in [
                ("get", true),
                ("update", config_writes && patch),
                ("reset", config_writes && reset),
                ("GET", false),
            ] {
                let request = BackendRequest::Advanced {
                    feature: feature.clone(),
                    action: action.into(),
                    body: if action == "update" {
                        json!({"updates":{"reflect_mission":"Synthetic mission"}})
                    } else {
                        json!({})
                    },
                };
                assert_eq!(capabilities.supports(&request), supported);
                if action == "get" {
                    backend
                        .execute(&scope(), request, CancellationToken::new())
                        .await
                        .unwrap();
                } else if !supported {
                    assert_eq!(
                        backend
                            .execute(&scope(), request, CancellationToken::new())
                            .await
                            .unwrap_err()
                            .code,
                        "unsupported_operation"
                    );
                }
            }
        }
        for (feature, action, supported) in [
            (AdvancedFeature::Directives, "get", true),
            (AdvancedFeature::Directives, "update", false),
            (AdvancedFeature::MentalModels, "create", worker && status),
            (AdvancedFeature::MentalModels, "refresh", worker && status),
            (AdvancedFeature::MentalModels, "update", true),
            (AdvancedFeature::MemoryEdit, "history", false),
            (AdvancedFeature::MemoryInvalidate, "history", false),
            (AdvancedFeature::MemoryRestore, "history", false),
            (AdvancedFeature::Documents, "delete", true),
        ] {
            assert_eq!(
                capabilities.supports(&BackendRequest::Advanced {
                    feature,
                    action: action.into(),
                    body: json!({})
                }),
                supported
            );
        }
        // Denied config mutations never leave the adapter; both reads still work.
        assert!(server
            .finish()
            .await
            .iter()
            .all(|request| request.method == "GET"));
    }
}
