use super::super::http::test_server::{serve, Reply, Server};
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
fn connection(endpoint: String, mode: &str) -> Connection {
    Connection {
        backend: BackendKind::OpenViking,
        name: "fixture".into(),
        endpoint: Some(endpoint),
        secret: None,
        embedding_profile: None,
        allow_insecure_http: true,
        options: Some(BackendOptions::OpenViking {
            bindings: BTreeMap::from([(
                "a".repeat(64),
                OpenVikingBinding {
                    mode: if mode == "user_key" {
                        OpenVikingAuthMode::UserKey
                    } else {
                        OpenVikingAuthMode::TrustedGateway
                    },
                    account_id: "fixture-account".into(),
                    user_id: "fixture-bot".into(),
                    api_key: "fixture-bot-secret".into(),
                },
            )]),
        }),
        extra: Default::default(),
    }
}
fn identity(mode: &str) -> Value {
    json!({"status":"ok","healthy":true,"version":"0.4.23","auth_mode":mode,"account_id":"fixture-account","user_id":"fixture-bot","role":"user"})
}
fn schema() -> Value {
    serde_json::from_str(include_str!("openviking_schema.json")).unwrap()
}
fn envelope(value: Value) -> Reply {
    Reply::json(json!({"status":"ok","result":value}))
}
async fn adapter(replies: Vec<Reply>, mode: &str) -> (OpenViking, Server) {
    let auth = if mode == "user_key" {
        "api_key"
    } else {
        "trusted"
    };
    let mut all = vec![
        Reply::json(identity(auth)),
        Reply::json(schema()),
        envelope(json!({"initialized":true,"user":"fixture-bot"})),
    ];
    all.extend(replies);
    let server = serve(all).await;
    let backend = OpenViking::connect(
        &connection(server.url.clone(), mode),
        &scope(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    (backend, server)
}
#[tokio::test]
async fn api_key_identity_cannot_be_switched_with_headers_or_root() {
    let (backend,server)=adapter(vec![Reply::json(identity("api_key")),envelope(json!({"memories":[{"uri":"viking://user/fixture-bot/memories/preferences/tea.md","abstract":"Synthetic tea preference"}],"resources":[],"skills":[]}))],"user_key").await;
    let result = backend
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
    assert_eq!(
        result.evidence[0].document_id.as_deref(),
        Some("memories/preferences/tea.md")
    );
    let seen = server.finish().await;
    for request in &seen {
        assert!(request
            .headers
            .to_lowercase()
            .contains("x-api-key: fixture-bot-secret"));
        assert!(!request.headers.to_lowercase().contains("x-openviking-user"));
        assert!(!request
            .headers
            .to_lowercase()
            .contains("x-openviking-account"));
    }
    assert_eq!(seen[4].path, "/api/v1/search/find");
    assert_eq!(seen[4].body["target_uri"], "viking://user/fixture-bot");
    for role in ["root", "admin"] {
        let mut health = identity("api_key");
        health["role"] = json!(role);
        let server = serve(vec![Reply::json(health)]).await;
        assert!(OpenViking::connect(
            &connection(server.url.clone(), "user_key"),
            &scope(),
            CancellationToken::new()
        )
        .await
        .is_err());
        server.finish().await;
    }
}
#[tokio::test]
async fn gateway_headers_require_explicit_trusted_mode_and_matching_identity() {
    let (_, server) = adapter(vec![], "trusted_gateway").await;
    let requests = server.finish().await;
    assert!(requests[0]
        .headers
        .to_lowercase()
        .contains("x-openviking-user: fixture-bot"));
    assert!(requests[0]
        .headers
        .to_lowercase()
        .contains("x-openviking-account: fixture-account"));
    for (mode, user) in [("api_key", "fixture-bot"), ("trusted", "foreign")] {
        let mut health = identity(mode);
        health["user_id"] = json!(user);
        let server = serve(vec![Reply::json(health)]).await;
        assert!(OpenViking::connect(
            &connection(server.url.clone(), "trusted_gateway"),
            &scope(),
            CancellationToken::new()
        )
        .await
        .is_err());
        server.finish().await;
    }
}
#[tokio::test]
async fn any_foreign_or_escaped_result_uri_fails_the_entire_recall() {
    for uri in [
        "viking://user/fixture-bot-foreign/memories/tea",
        "viking://user/other/memories/tea",
        "viking://user/fixture-bot/../other/tea",
        "viking://user/fixture-bot/%2e%2e/other/tea",
        "viking://~/memories/tea",
        "viking://resources/shared",
    ] {
        let (backend,server)=adapter(vec![Reply::json(identity("api_key")),envelope(json!({"memories":[{"uri":uri,"abstract":"foreign"}],"resources":[],"skills":[]}))],"user_key").await;
        assert_eq!(
            backend
                .execute(
                    &scope(),
                    BackendRequest::Recall {
                        query: "tea".into(),
                        budget: RecallBudget::default()
                    },
                    CancellationToken::new()
                )
                .await
                .unwrap_err()
                .code,
            "scope_mismatch"
        );
        server.finish().await;
    }
}
#[tokio::test]
async fn provenance_cannot_smuggle_a_foreign_scope() {
    let (backend,server)=adapter(vec![Reply::json(identity("api_key")),envelope(json!({"memories":[{"uri":"viking://user/fixture-bot/memories/tea","abstract":"fixture","provenance":{"source_uri":"viking://user/foreign/sessions/x"}}],"resources":[],"skills":[]}))],"user_key").await;
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
async fn session_commit_is_pending_until_task_completion() {
    let session = format!("beans-{}-doc", "a".repeat(64));
    let (backend,server)=adapter(vec![Reply::json(identity("api_key")),envelope(json!({"session_id":session,"uri":format!("viking://user/fixture-bot/sessions/{session}")})),
        envelope(json!({"session_id":session})),envelope(json!({"status":"accepted","task_id":"task","archive_uri":format!("viking://user/fixture-bot/sessions/{session}/history/archive_001"),"archived":true})),
        Reply::json(identity("api_key")),envelope(json!({"task_id":"task","status":"running"})),
        Reply::json(identity("api_key")),envelope(json!({"task_id":"task","status":"completed"})),
    ],"user_key").await;
    let result = backend
        .execute(
            &scope(),
            BackendRequest::Retain {
                document: FrozenDocument {
                    id: "doc".into(),
                    request_id: "request".into(),
                    text: "Synthetic fixture tea".into(),
                    sources: vec![],
                    content_hash: "hash".into(),
                },
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.operation.unwrap().state, OperationState::Submitted);
    for expected in [OperationState::Processing, OperationState::Completed] {
        let result = backend
            .execute(
                &scope(),
                BackendRequest::OperationStatus {
                    operation_id: "task".into(),
                },
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(result.operation.unwrap().state, expected);
    }
    let seen = server.finish().await;
    assert_eq!(seen[4].body["auto_commit_policy"], Value::Null);
    assert!(seen[4]
        .body
        .as_object()
        .unwrap()
        .contains_key("auto_commit_policy"));
    assert!(!backend.capabilities().idempotent_retain);
}
#[tokio::test]
async fn model_scope_arguments_and_identity_changes_fail_before_data_calls() {
    let (backend, server) = adapter(vec![], "user_key").await;
    let mut other = scope();
    other.namespace = "b".repeat(64);
    assert!(backend
        .execute(&other, BackendRequest::Health, CancellationToken::new())
        .await
        .is_err());
    for body in [
        json!({"uri":"viking://user/foreign"}),
        json!({"id":".."}),
        json!({"id":"x","user_id":"other"}),
    ] {
        assert!(backend
            .execute(
                &scope(),
                BackendRequest::Advanced {
                    feature: AdvancedFeature::Sessions,
                    action: "get".into(),
                    body
                },
                CancellationToken::new()
            )
            .await
            .is_err());
    }
    assert_eq!(server.finish().await.len(), 3);
    let mut changed = identity("api_key");
    changed["user_id"] = json!("foreign");
    let (backend, server) = adapter(vec![Reply::json(changed)], "user_key").await;
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
    assert_eq!(server.finish().await.len(), 4);
}
#[test]
fn bindings_are_validated_and_shared_user_keys_are_not_isolation() {
    let mut conn = connection("https://fixture.invalid".into(), "user_key");
    let Some(BackendOptions::OpenViking { bindings }) = &mut conn.options else {
        panic!()
    };
    let duplicate = bindings[&"a".repeat(64)].clone();
    bindings.insert("b".repeat(64), duplicate);
    assert!(validate_connection(&conn).is_err());
    let invalid = json!({"backend":"open_viking","bindings":{"a".repeat(64):{"mode":"user_key","account_id":"account","user_id":"user","api_key":"fixture","root_key":"forbidden"}}});
    assert!(serde_json::from_value::<BackendOptions>(invalid).is_err());
}

#[tokio::test]
async fn resources_force_own_user_target_and_disable_background_watches() {
    let (backend,server)=adapter(vec![Reply::json(identity("api_key")),envelope(json!({"task_id":"ingest-task","root_uri":"viking://user/fixture-bot/resources/beans/doc"})),
        Reply::json(identity("api_key")),envelope(json!({"task_id":"ingest-task","status":"failed","error":"fixture-bot-secret"}))],"user_key").await;
    let result = backend
        .execute(
            &scope(),
            BackendRequest::Advanced {
                feature: AdvancedFeature::Resources,
                action: "add".into(),
                body: json!({"id":"doc","source_url":"https://example.invalid/synthetic.md"}),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.operation.unwrap().state, OperationState::Submitted);
    let failed = backend
        .execute(
            &scope(),
            BackendRequest::OperationStatus {
                operation_id: "ingest-task".into(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(failed.operation.unwrap().state, OperationState::Failed);
    assert!(!failed.data.to_string().contains("fixture-bot-secret"));
    let seen = server.finish().await;
    assert_eq!(
        seen[4].body["to"],
        "viking://user/fixture-bot/resources/beans/doc"
    );
    assert_eq!(seen[4].body["watch_interval"], 0);
    assert_eq!(seen[4].body["wait"], false);
}

#[tokio::test]
async fn cancellation_is_cooperative_not_erasure_or_completed_storage() {
    let (backend,server)=adapter(vec![Reply::json(identity("api_key")),envelope(json!({"task_id":"task","status":"cancelling","resource_id":"viking://user/fixture-bot/resources/doc"}))],"user_key").await;
    let result = backend
        .execute(
            &scope(),
            BackendRequest::CancelOperation {
                operation_id: "task".into(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.operation.unwrap().state, OperationState::Processing);
    assert_eq!(result.data["quiescence_verified"], false);
    assert!(!backend.capabilities().write_fence);
    let seen = server.finish().await;
    assert_eq!(seen[4].path, "/api/v1/tasks/task/cancel");
}

#[tokio::test]
async fn recall_deadline_includes_identity_preflight() {
    let mut delayed = Reply::json(identity("api_key"));
    delayed.delay = Duration::from_millis(150);
    let (backend, server) = adapter(vec![delayed], "user_key").await;
    let budget = RecallBudget {
        timeout_ms: 30,
        ..RecallBudget::default()
    };
    let result = backend
        .execute(
            &scope(),
            BackendRequest::Recall {
                query: "fixture".into(),
                budget,
            },
            CancellationToken::new(),
        )
        .await;
    assert_eq!(result.unwrap_err().code, "service_timeout");
    assert_eq!(server.finish().await.len(), 4);
}

#[tokio::test]
async fn disabled_routes_are_not_inferred_from_package_version() {
    let mut actual = schema();
    actual["paths"]
        .as_object_mut()
        .unwrap()
        .remove("/api/v1/search/find");
    actual["paths"]
        .as_object_mut()
        .unwrap()
        .remove("/api/v1/tasks/{task_id}/cancel");
    let server = serve(vec![
        Reply::json(identity("api_key")),
        Reply::json(actual),
        envelope(json!({"initialized":true,"user":"fixture-bot"})),
    ])
    .await;
    let backend = OpenViking::connect(
        &connection(server.url.clone(), "user_key"),
        &scope(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(!backend.capabilities().recall);
    assert!(!backend.capabilities().cancel_operation);
    assert!(!backend.capabilities().reflect);
    assert!(backend
        .execute(
            &scope(),
            BackendRequest::Recall {
                query: "fixture".into(),
                budget: RecallBudget::default()
            },
            CancellationToken::new()
        )
        .await
        .is_err());
    assert_eq!(server.finish().await.len(), 3);
}

#[tokio::test]
async fn recalled_text_is_untrusted_content_not_an_absolute_uri_selector() {
    let (backend,server)=adapter(vec![Reply::json(identity("api_key")),envelope(json!({"memories":[{"uri":"viking://user/fixture-bot/memories/quoted.md","abstract":"viking://user/foreign is quoted synthetic content, not its provenance"}],"resources":[],"skills":[]}))],"user_key").await;
    let response = backend
        .execute(
            &scope(),
            BackendRequest::Recall {
                query: "fixture".into(),
                budget: RecallBudget::default(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.evidence[0].document_id.as_deref(),
        Some("memories/quoted.md")
    );
    assert!(response.evidence[0]
        .text
        .contains("quoted synthetic content"));
    server.finish().await;
}

#[tokio::test]
async fn canonical_resource_filenames_may_contain_spaces_and_unicode() {
    let (backend,server)=adapter(vec![Reply::json(identity("api_key")),envelope(json!({"memories":[],"resources":[{"uri":"viking://user/fixture-bot/resources/Team Notes/会議 (fixture).md","abstract":"Synthetic resource"}],"skills":[]}))],"user_key").await;
    let response = backend
        .execute(
            &scope(),
            BackendRequest::Recall {
                query: "fixture".into(),
                budget: RecallBudget::default(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.evidence[0].document_id.as_deref(),
        Some("resources/Team Notes/会議 (fixture).md")
    );
    server.finish().await;
}

#[tokio::test]
async fn advanced_actions_do_not_inherit_other_verbs_or_deprecated_permissions() {
    let mut actual = schema();
    let paths = actual["paths"].as_object_mut().unwrap();
    for (path, method) in [
        ("/api/v1/sessions", "get"),
        ("/api/v1/sessions/{session_id}", "delete"),
        ("/api/v1/content/read", "get"),
        ("/api/v1/tasks/{task_id}/cancel", "post"),
    ] {
        paths
            .get_mut(path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(method);
    }
    paths.get_mut("/api/v1/fs").unwrap()["delete"]["deprecated"] = json!(true);
    let server = serve(vec![
        Reply::json(identity("api_key")),
        Reply::json(actual),
        envelope(json!({"initialized":true,"user":"fixture-bot"})),
        Reply::json(identity("api_key")),
        envelope(json!({"session_id":"fixture"})),
    ])
    .await;
    let backend = OpenViking::connect(
        &connection(server.url.clone(), "user_key"),
        &scope(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let capabilities = backend.capabilities();
    for (feature, action, supported) in [
        (AdvancedFeature::Sessions, "create", true),
        (AdvancedFeature::Sessions, "get", true),
        (AdvancedFeature::Sessions, "list", false),
        (AdvancedFeature::Sessions, "delete", false),
        (AdvancedFeature::Resources, "list", true),
        (AdvancedFeature::Resources, "add", true),
        (AdvancedFeature::Resources, "read", false),
        (AdvancedFeature::Resources, "delete", false),
        (AdvancedFeature::Tasks, "get", true),
        (AdvancedFeature::Tasks, "list", true),
        (AdvancedFeature::Tasks, "cancel", false),
        (AdvancedFeature::Tasks, "GET", false),
    ] {
        let request = BackendRequest::Advanced {
            feature,
            action: action.into(),
            body: json!({"id":"fixture"}),
        };
        assert_eq!(capabilities.supports(&request), supported);
        if !supported {
            assert_eq!(
                backend
                    .execute(&scope(), request, CancellationToken::new())
                    .await
                    .unwrap_err()
                    .code,
                "unsupported_operation"
            );
        } else if action == "create" {
            backend
                .execute(&scope(), request, CancellationToken::new())
                .await
                .unwrap();
        }
    }
    // Denied actions do not perform even the per-operation identity preflight.
    assert_eq!(server.finish().await.len(), 5);
}
