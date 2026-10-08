use super::*;
use crate::embeddings::{EmbeddingBatch, EmbeddingError, EmbeddingFingerprint};
use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{Request, Response, StatusCode},
    Router,
};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
fn scope(bot: &str) -> MemoryScope {
    MemoryScope {
        namespace: namespace(&crate::keys::b64(&[11; 32]), bot).unwrap(),
        connection_id: "fixture".into(),
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
struct FixtureEmbedding {
    fingerprint: EmbeddingFingerprint,
}
#[async_trait::async_trait]
impl Embedding for FixtureEmbedding {
    fn fingerprint(&self) -> &EmbeddingFingerprint {
        &self.fingerprint
    }
    async fn embed(
        &self,
        _: EmbeddingPurpose,
        texts: &[String],
        _: CancellationToken,
    ) -> Result<EmbeddingBatch, EmbeddingError> {
        Ok(EmbeddingBatch {
            fingerprint: self.fingerprint.clone(),
            vectors: vec![vec![1., 0., 0.]; texts.len()],
        })
    }
}
fn provider() -> Arc<dyn Embedding> {
    Arc::new(FixtureEmbedding {
        fingerprint: EmbeddingFingerprint::parse(&"a".repeat(64)).unwrap(),
    })
}
struct Fixture {
    namespace: String,
    rows: Mutex<Vec<TransferRow>>,
    requests: Mutex<Vec<(String, serde_json::Value)>>,
    queries: AtomicUsize,
    transports: AtomicUsize,
    describe_generation: AtomicU64,
    query_generation: AtomicU64,
    foreign: bool,
}
async fn fixture(State(f): State<Arc<Fixture>>, request: Request<Body>) -> Response<Body> {
    f.transports.fetch_add(1, Ordering::SeqCst);
    assert_eq!(
        request.headers().get("x-api-key").unwrap(),
        "synthetic-secret"
    );
    let path = request.uri().path().to_string();
    let query = request.uri().query().unwrap_or("").to_string();
    let bytes = to_bytes(request.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    let mut output = Response::builder()
        .status(200)
        .header("phalanx-version", "0.3.0")
        .header("x-lancedb-version", "1");
    match path.as_str() {
        "/v1/table/beans_memory_v1/describe/" => {
            let mut described_space = space();
            described_space.generation = f.describe_generation.load(Ordering::SeqCst);
            let described_schema = schema(&described_space);
            let fields=described_schema.fields().iter().map(|field|{
                let typ=match field.data_type(){DataType::Utf8=>serde_json::json!({"type":"string"}),DataType::FixedSizeList(_,dim)=>serde_json::json!({"type":"fixed_size_list","length":dim,"fields":[{"name":"item","type":{"type":"float"},"nullable":true}]}),_=>panic!("unexpected type")};
                serde_json::json!({"name":field.name(),"type":typ,"nullable":field.is_nullable()})
            }).collect::<Vec<_>>();
            output = output.header("content-type", "application/json");
            output.body(Body::from(serde_json::json!({"version":1,"schema":{"fields":fields,"metadata":described_schema.metadata()}}).to_string())).unwrap()
        }
        "/v1/table/beans_memory_v1/merge_insert/" => {
            assert!(
                query.contains("namespace")
                    && query.contains("document_id")
                    && query.contains("chunk_id")
            );
            let reader =
                arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(bytes), None)
                    .unwrap();
            let batches = reader.collect::<Result<Vec<_>, _>>().unwrap();
            let incoming = decode(&space(), &f.namespace, &batches, 1000).unwrap();
            let mut rows = f.rows.lock().await;
            for row in incoming {
                rows.retain(|old| old.document.id != row.document.id);
                rows.push(row);
            }
            output.header("content-type","application/json").body(Body::from("{\"version\":1,\"num_updated_rows\":0,\"num_inserted_rows\":1,\"num_deleted_rows\":0}")).unwrap()
        }
        "/v1/table/beans_memory_v1/query/" => {
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let filter = body["filter"].as_str().unwrap();
            assert!(filter.contains(&f.namespace) && filter.contains(&space().fingerprint));
            if body["vector"].as_array().is_some_and(|vector| !vector.is_empty()) {
                assert_eq!(body["bypass_vector_index"], true);
                assert_eq!(body["prefilter"], true);
            }
            f.requests.lock().await.push((path, body));
            f.queries.fetch_add(1, Ordering::SeqCst);
            let ns = if f.foreign {
                scope("foreign").namespace
            } else {
                f.namespace.clone()
            };
            let mut returned_space = space();
            returned_space.generation = f.query_generation.load(Ordering::SeqCst);
            let batch = records(&returned_space, &ns, &f.rows.lock().await).unwrap();
            let mut ipc = Vec::new();
            let mut writer =
                arrow_ipc::writer::StreamWriter::try_new(&mut ipc, &batch.schema()).unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();
            drop(writer);
            output
                .header("content-type", "application/vnd.apache.arrow.stream")
                .body(Body::from(ipc))
                .unwrap()
        }
        "/v1/table/beans_memory_v1/delete/" => {
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(body["predicate"].as_str().unwrap().contains(&f.namespace));
            f.rows.lock().await.clear();
            output
                .header("content-type", "application/json")
                .body(Body::from("{\"version\":1,\"num_deleted_rows\":1}"))
                .unwrap()
        }
        _ => panic!("unexpected official SDK route: {path}"),
    }
}
async fn cloud_fixture(foreign: bool) -> (LanceBackend, Arc<Fixture>, tokio::task::JoinHandle<()>) {
    let a = scope("a");
    let f = Arc::new(Fixture {
        namespace: a.namespace.clone(),
        rows: Mutex::new(Vec::new()),
        requests: Mutex::new(Vec::new()),
        queries: AtomicUsize::new(0),
        transports: AtomicUsize::new(0),
        describe_generation: AtomicU64::new(1),
        query_generation: AtomicU64::new(1),
        foreign,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new().fallback(fixture).with_state(f.clone());
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let bridge = cloud::CloudBridge::start_to(
        reqwest::Url::parse(&format!("http://{address}/")).unwrap(),
        "synthetic-secret",
    )
    .await
    .unwrap();
    let connection = lancedb::connect("db://fixture")
        .region("us-east-1")
        .api_key(&bridge.token)
        .host_override(&bridge.url)
        .client_config(cloud_config())
        .read_consistency_interval(Duration::ZERO)
        .execute()
        .await
        .unwrap();
    let backend = LanceBackend {
        bound: a,
        space: space(),
        connection,
        embedding: Some(provider()),
        write: Mutex::new(()),
        local: false,
        _bridge: Some(bridge),
    };
    backend.ready().await.unwrap();
    (backend, f, handle)
}
#[tokio::test]
async fn cloud_real_sdk_scope_exact_search_provenance_curation_delete() {
    let (backend, f, server) = cloud_fixture(false).await;
    let a = scope("a");
    backend
        .execute(
            &a,
            BackendRequest::Retain {
                document: doc("id", "original"),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let response = backend
        .execute(
            &a,
            BackendRequest::Recall {
                query: "evidence".into(),
                budget: RecallBudget::default(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(response.evidence[0].text, "original");
    backend
        .execute(
            &a,
            BackendRequest::Advanced {
                feature: AdvancedFeature::MemoryEdit,
                action: "edit".into(),
                body: serde_json::json!({"document_id":"id","text":"curated"}),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let response = backend
        .execute(
            &a,
            BackendRequest::Inspect {
                document_id: "id".into(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .document
        .unwrap();
    assert_eq!(response.text, "curated");
    assert_eq!(response.sources[0].message_id, "id");
    let before = f.queries.load(Ordering::SeqCst);
    assert!(backend
        .execute(
            &scope("b"),
            BackendRequest::Recall {
                query: "foreign".into(),
                budget: RecallBudget::default()
            },
            CancellationToken::new()
        )
        .await
        .is_err());
    assert_eq!(f.queries.load(Ordering::SeqCst), before);
    assert!(backend
        .execute(
            &a,
            BackendRequest::Advanced {
                feature: AdvancedFeature::MemoryEdit,
                action: "edit".into(),
                body: serde_json::json!({"document_id":"id","text":"x","filter":"1=1"})
            },
            CancellationToken::new()
        )
        .await
        .is_err());
    backend
        .execute(
            &a,
            BackendRequest::Clear {
                request_id: "clear".into(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(backend
        .execute(
            &a,
            BackendRequest::Inspect {
                document_id: "id".into()
            },
            CancellationToken::new()
        )
        .await
        .unwrap()
        .document
        .is_none());
    server.abort();
}
#[tokio::test]
async fn cloud_refuses_foreign_rows_even_if_server_ignores_filter() {
    let (backend, _, server) = cloud_fixture(true).await;
    let a = scope("a");
    backend
        .retain_vector(
            &a,
            doc("id", "own"),
            &"a".repeat(64),
            vec![1., 0., 0.],
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        backend
            .recall_vector(
                &a,
                &"a".repeat(64),
                vec![1., 0., 0.],
                RecallBudget::default(),
                CancellationToken::new()
            )
            .await
            .unwrap_err()
            .code,
        "invalid_memory_response"
    );
    server.abort();
}
#[tokio::test]
async fn deletion_epoch_mismatch_rejects_all_lance_operations_before_transport() {
    let (backend, fixture, server) = cloud_fixture(false).await;
    let bound = scope("a");
    let saved = doc("id", "preserved");
    backend.execute(&bound, BackendRequest::Retain { document: saved.clone() }, CancellationToken::new()).await.unwrap();
    let mut changed = bound.clone();
    changed.deletion_epoch += 1;
    let before = fixture.transports.load(Ordering::SeqCst);
    let requests = vec![
        BackendRequest::Health,
        BackendRequest::Retain { document: doc("new", "rejected") },
        BackendRequest::Recall { query: "evidence".into(), budget: RecallBudget::default() },
        BackendRequest::Inspect { document_id: "id".into() },
        BackendRequest::DeleteDocument { document_id: "id".into(), request_id: "delete".into() },
        BackendRequest::Clear { request_id: "clear".into() },
        BackendRequest::Advanced { feature: AdvancedFeature::MemoryEdit, action: "edit".into(), body: serde_json::json!({"document_id":"id","text":"rejected"}) },
        BackendRequest::Reflect { query: "query".into(), budget: RecallBudget::default() },
        BackendRequest::OperationStatus { operation_id: "operation".into() },
        BackendRequest::CancelOperation { operation_id: "operation".into() },
    ];
    for request in requests {
        assert_eq!(backend.execute(&changed, request, CancellationToken::new()).await.unwrap_err().code, "memory_scope_mismatch");
    }
    assert_eq!(backend.retain_vector(&changed, doc("new", "rejected"), &space().fingerprint, vec![1.,0.,0.], CancellationToken::new()).await.unwrap_err().code, "memory_scope_mismatch");
    assert_eq!(backend.recall_vector(&changed, &space().fingerprint, vec![1.,0.,0.], RecallBudget::default(), CancellationToken::new()).await.unwrap_err().code, "memory_scope_mismatch");
    assert_eq!(backend.export(&changed, CancellationToken::new()).await.unwrap_err().code, "memory_scope_mismatch");
    let transfer = LanceTransfer { version: 1, namespace: bound.namespace.clone(), space: space(), rows: vec![TransferRow { document: saved.clone(), vector: vec![1.,0.,0.] }] };
    assert_eq!(backend.import(&changed, transfer, true, CancellationToken::new()).await.unwrap_err().code, "memory_scope_mismatch");
    assert_eq!(fixture.transports.load(Ordering::SeqCst), before);
    assert_eq!(fixture.rows.lock().await[0].document, saved);
    server.abort();
}

#[tokio::test]
async fn cloud_redirect_cannot_disclose_key_or_follow_to_attacker() {
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let attacker = listener.local_addr().unwrap();
    let target = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().fallback(move || {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    "attacker"
                }
            }),
        )
        .await
        .unwrap();
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let redirect = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().fallback(move || async move {
                Response::builder()
                    .status(StatusCode::TEMPORARY_REDIRECT)
                    .header("location", format!("http://{attacker}/secret"))
                    .body(Body::empty())
                    .unwrap()
            }),
        )
        .await
        .unwrap();
    });
    let bridge = cloud::CloudBridge::start_to(
        reqwest::Url::parse(&format!("http://{address}/")).unwrap(),
        "synthetic-secret",
    )
    .await
    .unwrap();
    let result = reqwest::Client::new()
        .post(format!("{}/v1/table/beans_memory_v1/describe/", bridge.url))
        .header("x-api-key", &bridge.token)
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    target.abort();
    redirect.abort();
}
#[test]
fn cloud_binding_and_document_literals_reject_target_selector_attacks() {
    assert!(cloud_target("db://fixture", "us-east-1", "key").is_ok());
    for uri in [
        "db://fixture/path",
        "db://user:password@fixture",
        "db://fixture?other=1",
        "http://fixture",
    ] {
        assert!(cloud_target(uri, "us-east-1", "key").is_err());
    }
    assert!(cloud_target("db://fixture", "us-east-1.evil", "key").is_err());
    assert_eq!(literal("x' OR 1=1 --").unwrap(), "'x'' OR 1=1 --'");
}

#[tokio::test]
async fn cloud_production_constructor_is_blocked_before_endpoint_use() {
    let result = LanceBackend::open(
        LanceBinding::Cloud { uri: "not-a-db-uri".into(), region: "invalid region".into(), api_key: "synthetic-secret".into() },
        scope("a"), space(), Some(provider()), true, CancellationToken::new(),
    ).await;
    match result {
        Ok(_) => panic!("unsafe Cloud constructor must not be enabled"),
        Err(error) => assert_eq!(error.code, "lancedb_cloud_transport_unavailable"),
    }
    for create in [false, true] {
        let result = LanceBackend::open(
            LanceBinding::Cloud { uri: "db://fixture".into(), region: "us-east-1".into(), api_key: "synthetic-secret".into() },
            scope("a"), space(), Some(provider()), create, CancellationToken::new(),
        ).await;
        match result {
            Ok(_) => panic!("valid Cloud configuration must remain transport-blocked"),
            Err(error) => assert_eq!(error.code, CLOUD_TRANSPORT_UNAVAILABLE),
        }
    }
}

#[tokio::test]
async fn returned_generation_mismatch_never_returns_evidence_or_document() {
    let (backend, fixture, server) = cloud_fixture(false).await;
    let bound = scope("a");
    backend.retain_vector(&bound, doc("id", "generation one"), &space().fingerprint, vec![1.,0.,0.], CancellationToken::new()).await.unwrap();
    fixture.query_generation.store(2, Ordering::SeqCst);
    for request in [
        BackendRequest::Recall { query: "evidence".into(), budget: RecallBudget::default() },
        BackendRequest::Inspect { document_id: "id".into() },
        BackendRequest::Advanced { feature: AdvancedFeature::MemoryEdit, action: "edit".into(), body: serde_json::json!({"document_id":"id","text":"rejected"}) },
    ] {
        assert!(backend.execute(&bound, request, CancellationToken::new()).await.is_err(), "changed Arrow generation must not be accepted");
    }
    assert_eq!(fixture.rows.lock().await[0].document.text, "generation one");
    server.abort();
}

#[tokio::test]
async fn descriptor_generation_change_bypasses_no_schema_cache_for_writes() {
    let (backend, fixture, server) = cloud_fixture(false).await;
    let bound = scope("a");
    backend.retain_vector(&bound, doc("id", "generation one"), &space().fingerprint, vec![1.,0.,0.], CancellationToken::new()).await.unwrap();
    fixture.describe_generation.store(2, Ordering::SeqCst);
    let before = fixture.rows.lock().await[0].document.clone();
    assert!(backend.retain_vector(&bound, doc("id", "rejected"), &space().fingerprint, vec![1.,0.,0.], CancellationToken::new()).await.is_err(), "SDK cached descriptor must not authorize changed-generation mutation");
    assert_eq!(fixture.rows.lock().await[0].document, before);
    server.abort();
}

async fn seed_collision_row(backend: &LanceBackend, namespace: &str, fingerprint: &str, document: FrozenDocument) {
    let table = backend.connection.open_table(TABLE).execute().await.unwrap();
    let batch = records(&backend.space, namespace, &[TransferRow { document, vector: vec![1.,0.,0.] }]).unwrap();
    let mut columns = batch.columns().to_vec();
    let field = batch.schema().index_of("fingerprint").unwrap();
    columns[field] = Arc::new(StringArray::from(vec![fingerprint]));
    let batch = RecordBatch::try_new(batch.schema(), columns).unwrap();
    table.add(batch).execute().await.unwrap();
}

#[tokio::test]
async fn fingerprint_collision_never_acknowledges_skipped_retain() {
    let directory = tempfile::tempdir().unwrap();
    let bound = scope("a");
    let sibling = scope("b");
    let backend = LanceBackend::open(LanceBinding::Local { directory: directory.path().join("db"), runner_id: "runner".into() }, bound.clone(), space(), None, true, CancellationToken::new()).await.unwrap();
    seed_collision_row(&backend, &bound.namespace, &"b".repeat(64), doc("doc", "other space")).await;
    seed_collision_row(&backend, &sibling.namespace, &space().fingerprint, doc("doc", "sibling")).await;
    let requested = doc("doc", "requested evidence");
    let result = backend.retain_vector(&bound, requested.clone(), &space().fingerprint, vec![1.,0.,0.], CancellationToken::new()).await;
    if result.is_ok() {
        assert_eq!(backend.inspect(&bound, "doc", CancellationToken::new()).await.unwrap().document, Some(requested), "acknowledged retain must actually store the requested A-space document");
    }
    let other = LanceBackend::open(LanceBinding::Local { directory: directory.path().join("db"), runner_id: "runner".into() }, sibling.clone(), space(), None, false, CancellationToken::new()).await.unwrap();
    assert_eq!(other.inspect(&sibling, "doc", CancellationToken::new()).await.unwrap().document.unwrap().text, "sibling");
}

#[tokio::test]
async fn fingerprint_collision_never_acknowledges_skipped_import() {
    let directory = tempfile::tempdir().unwrap();
    let bound = scope("a");
    let backend = LanceBackend::open(LanceBinding::Local { directory: directory.path().join("db"), runner_id: "runner".into() }, bound.clone(), space(), None, true, CancellationToken::new()).await.unwrap();
    seed_collision_row(&backend, &bound.namespace, &"b".repeat(64), doc("doc", "other space")).await;
    let requested = doc("doc", "requested import");
    let transfer = LanceTransfer { version: 1, namespace: bound.namespace.clone(), space: space(), rows: vec![TransferRow { document: requested.clone(), vector: vec![1.,0.,0.] }] };
    let result = backend.import(&bound, transfer, true, CancellationToken::new()).await;
    if result.is_ok() {
        assert_eq!(backend.inspect(&bound, "doc", CancellationToken::new()).await.unwrap().document, Some(requested), "acknowledged import must actually store the requested A-space document");
    }
}

#[tokio::test]
async fn vector_outer_deadline_preserves_read_write_uncertainty() {
    for (write, code) in [(false, "memory_timeout"), (true, "delivery_unknown")] {
        // Work cannot finish, so the controlled zero deadline has no work/timer race.
        let error = bounded(
            CancellationToken::new(),
            0,
            write,
            std::future::pending::<Result<(), MemoryError>>(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, code);
    }
}

#[tokio::test]
async fn vector_precancellation_does_not_poll_work() {
    for write in [false, true] {
        let token = CancellationToken::new();
        token.cancel();
        let polls = AtomicUsize::new(0);
        let work = std::future::poll_fn(|_| {
            polls.fetch_add(1, Ordering::SeqCst);
            std::task::Poll::Ready(Ok::<(), MemoryError>(()))
        });
        let error = bounded(token, 5000, write, work).await.unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert_eq!(polls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn vector_inflight_cancellation_wins_over_ready_work() {
    use std::future::Future;
    use std::task::{Context, Poll, Wake, Waker};
    struct Noop;
    impl Wake for Noop {
        fn wake(self: Arc<Self>) {}
    }
    for (write, code) in [(false, "cancelled"), (true, "delivery_unknown")] {
        let token = CancellationToken::new();
        let (started_tx, mut started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let work = async move {
            started_tx.send(()).unwrap();
            release_rx.await.unwrap();
            Ok::<(), MemoryError>(())
        };
        let mut boundary = Box::pin(bounded(token.clone(), 5000, write, work));
        let waker = Waker::from(Arc::new(Noop));
        let mut cx = Context::from_waker(&waker);
        assert!(matches!(boundary.as_mut().poll(&mut cx), Poll::Pending));
        assert_eq!(started_rx.try_recv(), Ok(()));
        // Polling reached work; make completion and cancellation ready together.
        release_tx.send(()).unwrap();
        token.cancel();
        let error = match boundary.as_mut().poll(&mut cx) {
            Poll::Ready(Err(error)) => error,
            _ => panic!("biased cancellation must win"),
        };
        assert_eq!(error.code, code);
    }
}

#[tokio::test]
async fn vector_ready_error_is_not_reclassified_as_outer_timeout() {
    for write in [false, true] {
        let error = bounded(
            CancellationToken::new(),
            5000,
            write,
            std::future::ready(Err::<(), _>(MemoryError::new("lance_query_failed"))),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "lance_query_failed");
    }
}

#[tokio::test]
async fn vector_returned_sdk_failure_is_sanitized_and_success_is_preserved() {
    struct SecretSdkFailure;
    impl std::fmt::Display for SecretSdkFailure {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("raw SDK error must never be formatted");
        }
    }
    impl std::fmt::Debug for SecretSdkFailure {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("raw SDK error must never be debug-formatted");
        }
    }
    let error = bounded(
        CancellationToken::new(),
        5000,
        true,
        std::future::ready(merge_result(Err::<(), _>(SecretSdkFailure))),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "delivery_unknown");
    let private_detail = "synthetic-sdk-secret: /private/fixture/request-text";
    let private_error = merge_result(Err::<(), _>(private_detail)).unwrap_err();
    assert_eq!(private_error.code, "delivery_unknown");
    assert!(!private_error.message.contains("synthetic-sdk-secret"));
    assert!(!private_error.message.contains("/private/fixture"));
    assert!(!private_error.message.contains("request-text"));
    let value = bounded(
        CancellationToken::new(),
        5000,
        true,
        std::future::ready(merge_result::<_, SecretSdkFailure>(Ok(42))),
    )
    .await
    .unwrap();
    assert_eq!(value, 42);
}
