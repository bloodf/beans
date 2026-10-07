use std::sync::Arc;

use super::*;
use lorca::app::OutboxItem;
use lorca::relay::RelayClient;

struct Relay {
    state: AppState,
    url: String,
    token: String,
    identity: String,
    machine: String,
    client: RelayClient,
    http: reqwest::Client,
    home: std::path::PathBuf,
    task: tokio::task::JoinHandle<()>,
}

impl Relay {
    async fn start(quota_bytes: u64) -> Self {
        Self::start_with(quota_bytes, crate::push::Pusher::new(None, None)).await
    }

    async fn start_with(quota_bytes: u64, pusher: crate::push::Pusher) -> Self {
        Self::start_catalogs(quota_bytes, pusher, None).await
    }

    async fn start_catalogs(quota_bytes: u64, pusher: crate::push::Pusher, files: Option<(&[u8], &[u8])>) -> Self {
        Self::start_protocol(quota_bytes, pusher, files, PROTOCOL).await
    }

    async fn start_protocol(quota_bytes: u64, pusher: crate::push::Pusher, files: Option<(&[u8], &[u8])>, min_protocol: u32) -> Self {
        Self::start_database(quota_bytes, pusher, files, min_protocol, None).await
    }

    async fn start_database(quota_bytes: u64, pusher: crate::push::Pusher, files: Option<(&[u8], &[u8])>,
        min_protocol: u32, database: Option<&str>) -> Self {
        let home = std::env::temp_dir().join(format!("lorca-binary-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let catalog_dir = home.join("catalogs");
        if let Some((models, marketplace)) = files {
            std::fs::create_dir_all(catalog_dir.join("models")).unwrap();
            std::fs::create_dir_all(catalog_dir.join("marketplace")).unwrap();
            std::fs::write(catalog_dir.join("models/v1.json"), models).unwrap();
            std::fs::write(catalog_dir.join("marketplace/v1.json"), marketplace).unwrap();
        }
        let local = Arc::new(db::Local::default());
        let path = home.join("relay.db");
        let db = db::open(database.unwrap_or(path.to_str().unwrap()), local.clone()).await.unwrap();
        let (identity, machine) = if database.is_some() {
            let suffix = uuid::Uuid::new_v4();
            (format!("identity-{suffix}"), format!("machine-{suffix}"))
        } else { ("identity".into(), "machine".into()) };
        db.register_identity(&identity, "content", &machine, "box", "attestation").await.unwrap();
        let state = AppState {
            db,
            local,
            secret: Arc::new([7; 32]),
            quota_bytes,
            ip_limiter: Arc::new(crate::limit::RateLimiter::new(0.0, 1)),
            identity_limiter: Arc::new(crate::limit::RateLimiter::new(0.0, 1)),
            trust_proxy: false,
            uploads: Some(Arc::new(tokio::sync::Semaphore::new(1))),
            min_protocol,
            catalogs: Catalogs::load(Some(&catalog_dir)),
            metrics_token: None,
            stats: Arc::new(crate::metrics::StatsCache::default()),
            instance: "test".into(),
            stopping: tokio_util::sync::CancellationToken::new(),
            file_store: Arc::new(crate::store::FileStore::Local { dir: home.join("files") }),
            pusher: Arc::new(pusher),
            pushes: tokio_util::task::TaskTracker::new(),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = router(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let token = issue_token(&state.secret, &identity, &machine).0;
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("lorca-protocol", PROTOCOL.into());
        let http = reqwest::Client::builder().default_headers(headers).build().unwrap();
        Self {
            state,
            url,
            token,
            identity,
            machine,
            client: RelayClient::new().unwrap(),
            http,
            home,
            task,
        }
    }

    fn put(&self, id: &str) -> reqwest::RequestBuilder {
        self.http
            .put(format!("{}/v1/files/{id}", self.url))
            .bearer_auth(&self.token)
            .header(header::CONTENT_TYPE, "application/octet-stream")
    }

    async fn upload(&self, id: &str, group: Option<&str>, ciphertext: Vec<u8>) -> i64 {
        self.client.put_blob(&self.url, &self.token, file(id, group, ciphertext), 0).await.unwrap()
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn file(id: &str, group: Option<&str>, ciphertext: Vec<u8>) -> OutboxItem {
    OutboxItem {
        id: id.into(),
        kind: "file".into(),
        recipient: None,
        ciphertext,
        slot: None,
        group: group.map(str::to_string),
    }
}

#[tokio::test]
async fn public_catalogs_are_plaintext_cached_and_independent_of_account_protocol() {
    let models = br#"{"models":[{"id":"public-model"}]}"#;
    let marketplace = br#"{"plugins":[{"id":"public-plugin"}]}"#;
    let relay = Relay::start_catalogs(0, crate::push::Pusher::new(None, None), Some((models, marketplace))).await;
    let old = reqwest::Client::builder().default_headers({
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("lorca-protocol", 2.into());
        headers
    }).build().unwrap();
    for (route, expected) in [("/models/v1.json", models.as_slice()), ("/marketplace/v1.json", marketplace.as_slice())] {
        let response = old.get(format!("{}{route}", relay.url)).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "public, max-age=300, must-revalidate");
        let etag = response.headers()[header::ETAG].to_str().unwrap().to_owned();
        let body = response.bytes().await.unwrap();
        assert_eq!(body.as_ref(), expected);
        assert!(!body.windows(b"identity".len()).any(|part| part == b"identity"));
        let cached = old.get(format!("{}{route}", relay.url)).header(header::IF_NONE_MATCH, etag).send().await.unwrap();
        assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
        assert!(cached.bytes().await.unwrap().is_empty());
    }
    assert_eq!(old.get(format!("{}/v1/machines", relay.url)).send().await.unwrap().status(), StatusCode::UPGRADE_REQUIRED);
    assert_eq!(reqwest::get(format!("{}/models/v1.json", relay.url)).await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn unavailable_catalogs_fail_without_exposing_account_data() {
    let missing = Relay::start(0).await;
    for route in ["/models/v1.json", "/marketplace/v1.json"] {
        let response = reqwest::get(format!("{}{route}", missing.url)).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(!response.text().await.unwrap().contains("identity"));
    }
    let malformed = Relay::start_catalogs(0, crate::push::Pusher::new(None, None), Some((b"not JSON", b"{}"))).await;
    assert_eq!(reqwest::get(format!("{}/models/v1.json", malformed.url)).await.unwrap().status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reqwest::get(format!("{}/marketplace/v1.json", malformed.url)).await.unwrap().status(), StatusCode::OK);
    assert!(matches!(Catalog::load(Some(&malformed.home.join("catalogs/models/v1.json"))), Catalog::Invalid));
    let large = malformed.home.join("catalogs/models/v1.json");
    std::fs::write(&large, vec![b' '; MAX_CATALOG_BYTES as usize + 1]).unwrap();
    assert!(matches!(Catalog::load(Some(&large)), Catalog::Invalid));
}

#[tokio::test]
async fn roster_put_requires_expected_slot_seq_and_reports_conflict() {
    let relay = Relay::start(0).await;
    let put = |id: &str, expected: Option<i64>| {
        let mut body = json!({ "id": id, "kind": "roster", "slot": "roster", "ciphertext": "AQ" });
        if let Some(expected) = expected { body["expected_slot_seq"] = expected.into(); }
        relay.http.put(format!("{}/v1/blobs", relay.url)).bearer_auth(&relay.token).json(&body)
    };
    assert_eq!(put("missing", None).send().await.unwrap().status(), StatusCode::BAD_REQUEST);
    let first = put("one", Some(0)).send().await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let seq = first.json::<Value>().await.unwrap()["seq"].as_i64().unwrap();
    assert_eq!(put("two", Some(0)).send().await.unwrap().status(), StatusCode::CONFLICT);
    let existing = put("one", Some(0)).send().await.unwrap();
    assert_eq!(existing.status(), StatusCode::OK);
    let body = existing.json::<Value>().await.unwrap();
    assert_eq!(body["seq"], seq);
    assert_eq!(body["existing"], true);
    let next = put("two", Some(seq)).send().await.unwrap();
    assert_eq!(next.status(), StatusCode::OK);
    assert_eq!(next.json::<Value>().await.unwrap()["seq"], seq + 1);
}

#[tokio::test]
async fn appearance_roster_floor_cannot_be_lowered_and_old_writers_have_no_effects() {
    let relay = Relay::start_protocol(0, crate::push::Pusher::new(None, None), None, 3).await;
    let health = reqwest::get(format!("{}/v1/health", relay.url)).await.unwrap().json::<Value>().await.unwrap();
    assert_eq!(health["protocol"], 4);
    assert_eq!(health["min_protocol"], 3);
    assert_eq!(health["min_roster_protocol"], 4);
    assert_eq!(health["memory_config_version"], 1);
    assert_eq!(relay.client.health(&relay.url).await.unwrap(), 4);
    let body = json!({"id":"capable","kind":"roster","slot":"roster","expected_slot_seq":0,"ciphertext":"AQ"});
    let response = relay.http.put(format!("{}/v1/blobs", relay.url)).header("lorca-protocol", "4")
        .bearer_auth(&relay.token).json(&body).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let seq = response.json::<Value>().await.unwrap()["seq"].as_i64().unwrap();
    let usage = relay.state.db.stats().await.unwrap().usage_bytes;
    for header in ["3", "2", "0", "garbage", "4294967296"] {
        let mut old = body.clone();
        old["id"] = json!("old");
        old["expected_slot_seq"] = json!(seq);
        let response = relay.http.put(format!("{}/v1/blobs", relay.url)).header("lorca-protocol", header)
            .bearer_auth(&relay.token).json(&old).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED, "{header}");
        assert!(relay.state.db.blob("identity", "machine", "old").await.unwrap().is_none());
        assert_eq!(relay.state.db.blob("identity", "machine", "capable").await.unwrap().unwrap().seq, seq);
        assert_eq!(relay.state.db.stats().await.unwrap().usage_bytes, usage);
    }
    let no_header = reqwest::Client::new().put(format!("{}/v1/blobs", relay.url)).bearer_auth(&relay.token)
        .json(&body).send().await.unwrap();
    assert_eq!(no_header.status(), StatusCode::UPGRADE_REQUIRED);
    // A lower generic minimum still permits protocol-3 account reads, not roster replacement.
    assert_eq!(relay.http.get(format!("{}/v1/machines", relay.url)).header("lorca-protocol", "3")
        .bearer_auth(&relay.token).send().await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn protected_blob_delete_rejects_protocol3_and4_without_row_sequence_or_usage_changes() {
    let mut databases = vec![None];
    if let Ok(url) = std::env::var("LORCA_RELAY_TEST_POSTGRES") { databases.push(Some(url)); }
    for database in databases {
        let relay = Relay::start_database(0, crate::push::Pusher::new(None, None), None, 3, database.as_deref()).await;
        for (id, kind, ciphertext) in [("roster", "roster", "bG9vaw"), ("policy", "policy", "cGF1c2U"), ("memory_config","memory_config","AQ")] {
            let mut body = json!({"id":id,"kind":kind,"ciphertext":ciphertext});
            if kind == "roster" { body["slot"] = json!("roster"); body["expected_slot_seq"] = json!(0); }
            if kind=="memory_config" {body["slot"]=json!("memory_config");}
            let response = relay.http.put(format!("{}/v1/blobs", relay.url)).bearer_auth(&relay.token)
                .json(&body).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        let before = relay.http.get(format!("{}/v1/blobs?since=0", relay.url)).bearer_auth(&relay.token)
            .send().await.unwrap().json::<Value>().await.unwrap();
        for protocol in [3, 4] {
            for id in ["roster", "policy", "memory_config"] {
                let response = relay.http.delete(format!("{}/v1/blobs/{id}", relay.url)).header("lorca-protocol", protocol)
                    .bearer_auth(&relay.token).send().await.unwrap();
                assert_eq!(response.status(), StatusCode::NOT_FOUND, "protocol {protocol}: {id}");
                let after = relay.http.get(format!("{}/v1/blobs?since=0", relay.url)).bearer_auth(&relay.token)
                    .send().await.unwrap().json::<Value>().await.unwrap();
                assert_eq!(after, before, "protocol {protocol}: {id} changed rows or head sequence");
                assert!(relay.state.db.precheck_blob(&relay.identity, "quota-probe", None, 1, 11).await.is_ok());
                assert!(relay.state.db.precheck_blob(&relay.identity, "quota-probe", None, 2, 11).await.is_err(),
                    "protocol {protocol}: {id} changed stored usage");
            }
        }
        for protocol in [3, 4] {
            let id = format!("consumed-{protocol}");
            let response = relay.http.put(format!("{}/v1/blobs", relay.url)).header("lorca-protocol", protocol)
                .bearer_auth(&relay.token).json(&json!({"id":id,"kind":"chat","ciphertext":"b2s"})).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let response = relay.http.delete(format!("{}/v1/blobs/{id}", relay.url)).header("lorca-protocol", protocol)
                .bearer_auth(&relay.token).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            assert!(relay.state.db.blob(&relay.identity, &relay.machine, &id).await.unwrap().is_none());
            assert!(relay.state.db.precheck_blob(&relay.identity, "quota-probe", None, 1, 11).await.is_ok());
            assert!(relay.state.db.precheck_blob(&relay.identity, "quota-probe", None, 2, 11).await.is_err());
        }
    }
}

#[tokio::test]
async fn appearance_health_fails_closed_without_an_enforced_supported_roster_floor() {
    for health in [
        json!({"protocol":3,"min_protocol":3,"min_roster_protocol":3}),
        json!({"protocol":4}),
        json!({"protocol":4,"min_protocol":3,"min_roster_protocol":3}),
        json!({"protocol":4,"min_protocol":5,"min_roster_protocol":5}),
        json!({"protocol":4,"min_protocol":3,"min_roster_protocol":5}),
        json!({"protocol":4294967300_u64,"min_protocol":3,"min_roster_protocol":4}),
        json!({"protocol":4,"min_protocol":3,"min_roster_protocol":"4"}),
        json!({"protocol":4,"min_protocol":null,"min_roster_protocol":4}),
        json!({"protocol":4,"min_protocol":3,"min_roster_protocol":4}),
        json!({"protocol":4,"min_protocol":3,"min_roster_protocol":4,"memory_config_version":1}),
        json!({"protocol":4,"min_protocol":3,"min_roster_protocol":4,"memory_config_version":"1"}),
        json!({"protocol":4,"min_protocol":3,"min_roster_protocol":4,"memory_config_version":2}),
    ] {
        let expected = health == json!({"protocol":4,"min_protocol":3,"min_roster_protocol":4,"memory_config_version":1});
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let response = health.clone();
        let app = Router::new().route("/v1/health", get(move || { let response = response.clone(); async move { Json(response) } }));
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let result = RelayClient::new().unwrap().health(&url).await;
        task.abort();
        assert_eq!(result.is_ok(), expected, "{health}: {result:?}");
        if !expected { assert_eq!(result.unwrap_err().status, Some(426)); }
    }
}

#[tokio::test]
async fn binary_attachment_round_trip_stays_encrypted_and_idempotent() {
    let relay = Relay::start(0).await;
    let plaintext = b"attachment\0\xff\xfe\x80\r\n";
    let ciphertext = lorca::crypto::encrypt(&[9; 32], "file", plaintext).unwrap();
    assert_eq!(ciphertext.len(), plaintext.len() + lorca::crypto::ENVELOPE_OVERHEAD);
    let seq = relay.upload("att-file", Some("chat"), ciphertext.clone()).await;
    assert_eq!(relay.upload("att-file", Some("chat"), vec![0; 10]).await, seq);
    let downloaded = relay.client.get_file(&relay.url, &relay.token, "att-file").await.unwrap().unwrap();
    assert_eq!(downloaded, ciphertext);
    assert_eq!(lorca::crypto::decrypt(&[9; 32], "file", &downloaded).unwrap(), plaintext);
    assert_eq!(std::fs::read(relay.home.join("files/identity/att-file")).unwrap(), ciphertext);
    let row = relay.state.db.blob("identity", "machine", "att-file").await.unwrap().unwrap();
    assert!(row.ciphertext.is_empty());
    let page = relay.client.list_blobs(&relay.url, &relay.token, 0, "").await.unwrap();
    assert!(page.0.is_empty(), "JSON sync pages contain no attachment payloads");
    let chat = OutboxItem {
        id: "message".into(),
        kind: "chat".into(),
        recipient: None,
        ciphertext: vec![0, 255, 128],
        slot: None,
        group: Some("chat".into()),
    };
    relay.client.put_blob(&relay.url, &relay.token, chat, 0).await.unwrap();
    let (blobs, _) = relay.client.list_blobs(&relay.url, &relay.token, 0, "").await.unwrap();
    assert_eq!(blobs.len(), 1);
    assert_eq!(lorca::keys::unb64(&blobs[0].ciphertext).unwrap(), [0, 255, 128]);
    assert_eq!(relay.client.get_file(&relay.url, &relay.token, "message").await.unwrap(), None);

    let other = issue_token(&relay.state.secret, "other-identity", "other-machine").0;
    assert!(relay.client.get_file(&relay.url, &other, "att-file").await.unwrap().is_none());
    relay.client.delete_group(&relay.url, &relay.token, "chat").await.unwrap();
    assert!(relay.client.get_file(&relay.url, &relay.token, "att-file").await.unwrap().is_none());
    assert!(!relay.home.join("files/identity/att-file").exists());
    let error = relay
        .client
        .put_blob(&relay.url, &relay.token, file("att-late", Some("chat"), vec![1]), 0)
        .await
        .unwrap_err();
    assert_eq!(error.status, Some(409));
    assert!(!relay.home.join("files/identity/att-late").exists());
}

#[tokio::test]
async fn binary_files_enforce_auth_metadata_quota_and_missing_objects() {
    let relay = Relay::start(4).await;
    let unauthenticated = relay
        .http
        .put(format!("{}/v1/files/att-file", relay.url))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(vec![1])
        .send()
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(relay.put("att-empty").body(Vec::new()).send().await.unwrap().status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        relay
            .http
            .put(format!("{}/v1/files/att-file", relay.url))
            .bearer_auth(&relay.token)
            .header(header::CONTENT_TYPE, "application/json")
            .body(vec![1])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        relay.put("att-file").query(&[("group", "../bad")]).body(vec![1]).send().await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        relay
            .put("att-file")
            .query(&[("slot", "not-allowed")])
            .body(vec![1])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    let error = relay
        .client
        .put_blob(&relay.url, &relay.token, file("att-large", None, vec![1; 5]), 0)
        .await
        .unwrap_err();
    assert_eq!(error.status, Some(413));
    assert!(!relay.home.join("files/identity/att-large").exists());

    relay.upload("att-avatar", None, vec![0, 255, 128, 1]).await;
    std::fs::remove_file(relay.home.join("files/identity/att-avatar")).unwrap();
    assert!(relay.client.get_file(&relay.url, &relay.token, "att-avatar").await.unwrap().is_none());
    assert!(relay.state.db.blob("identity", "machine", "att-avatar").await.unwrap().is_some());
    relay.client.delete_blob(&relay.url, &relay.token, "att-avatar").await.unwrap();
    relay.upload("att-replacement", None, vec![2; 4]).await;

    let response = relay
        .http
        .put(format!("{}/v1/blobs", relay.url))
        .bearer_auth(&relay.token)
        .json(&json!({ "id": "att-json", "kind": "file", "ciphertext": "AQ" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = relay.put("att-old").header("lorca-protocol", "0").body(vec![1]).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
}

#[tokio::test]
async fn binary_file_limit_includes_the_encryption_envelope_and_caps_chunked_bodies() {
    let relay = Relay::start(0).await;
    assert_eq!(
        MAX_FILE_BLOB_BYTES,
        lorca::files::MAX_ATTACHMENT_BYTES as usize + lorca::crypto::ENVELOPE_OVERHEAD
    );
    relay.upload("att-boundary", None, vec![0x80; MAX_FILE_BLOB_BYTES]).await;
    let downloaded = relay.client.get_file(&relay.url, &relay.token, "att-boundary").await.unwrap().unwrap();
    assert_eq!(downloaded.len(), MAX_FILE_BLOB_BYTES);
    assert!(downloaded.iter().all(|&byte| byte == 0x80));
    drop(downloaded);
    let chunks = futures::stream::iter((0..101).map(|_| Ok::<_, std::io::Error>(Bytes::from(vec![0; 1024 * 1024]))));
    let response = relay.put("att-overflow").body(reqwest::Body::wrap_stream(chunks)).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!relay.home.join("files/identity/att-overflow").exists());
}

/// A relay told to stop still hands APNs the pushes it answered `queued` for, and stops waiting
/// for one APNs never takes once the grace is up.
#[tokio::test]
async fn a_stopping_relay_delivers_the_pushes_it_took() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// A relay whose identity has a phone that takes pushes at `phone_token`.
    async fn relay_with_phone(apns_url: &str, phone_token: &str) -> Relay {
        let apns = crate::push::Apns::new(crate::push::tests::P8, "KEYID12345".into(), "TEAMID1234".into(), "app.lorca".into(), Some(apns_url.into())).unwrap();
        let relay = Relay::start_with(0, crate::push::Pusher::new(Some(apns), None)).await;
        relay.state.db.register_identity("identity", "content", "phone", "phone-box", "attestation").await.unwrap();
        let token = db::PushToken { machine_pubkey: "phone".into(), platform: "apns".into(), token: phone_token.into(), environment: "sandbox".into() };
        relay.state.db.set_push_token("identity", &token).await.unwrap();
        relay
    }

    // APNs takes a while over the token `slow`, and never answers for `stuck`.
    let delivered = Arc::new(AtomicUsize::new(0));
    let apns = Router::new().route("/3/device/{token}", post({
        let delivered = delivered.clone();
        move |Path(token): Path<String>| async move {
            if token == "stuck" {
                std::future::pending::<()>().await;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            delivered.fetch_add(1, Ordering::Relaxed);
            StatusCode::OK
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let apns_url = format!("http://{}", listener.local_addr().unwrap());
    let apns = tokio::spawn(async move { axum::serve(listener, apns).await.unwrap() });

    let relay = relay_with_phone(&apns_url, "slow").await;
    assert_eq!(relay.client.push(&relay.url, &relay.token, &b64url_encode(b"notice")).await.unwrap(), 1);
    crate::stop(&relay.state, Duration::from_secs(5)).await;
    assert_eq!(delivered.load(Ordering::Relaxed), 1, "the push reached APNs before the relay stopped");

    let relay = relay_with_phone(&apns_url, "stuck").await;
    assert_eq!(relay.client.push(&relay.url, &relay.token, &b64url_encode(b"notice")).await.unwrap(), 1);
    let started = Instant::now();
    crate::stop(&relay.state, Duration::from_millis(300)).await;
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(300) && waited < Duration::from_secs(3), "waited {waited:?}");
    assert_eq!(delivered.load(Ordering::Relaxed), 1);
    apns.abort();
}

/// A computer that joined by pairing pairs the next one: the identity device attests the first
/// with the identity key, that one attests the next with its machine key, and the newest gets
/// the account. Once unpaired, a computer pairs nobody.
#[tokio::test]
async fn a_paired_computer_pairs_another() {
    let relay = Relay::start(0).await;
    let app = |name: &str| lorca::app::App::load(lorca::config::Config { home: relay.home.join(name), port: 0 }).unwrap();
    let (first, second, third) = (app("first"), app("second"), app("third"));
    lorca::identity::create(&first, Some("First".into())).unwrap();
    first.set_relay_url(Some(relay.url.clone())).unwrap();

    let (_, code) = lorca::pairing::start(first.clone()).await.unwrap();
    lorca::pairing::accept(second.clone(), &code, Some("Second".into())).await.unwrap();
    assert!(!second.is_identity_device());

    let (nonce, code) = lorca::pairing::start(second.clone()).await.unwrap();
    lorca::pairing::accept(third.clone(), &code, Some("Third".into())).await.unwrap();
    let (account, joined) = (first.machine_file().unwrap(), third.machine_file().unwrap());
    assert_eq!(joined.identity_pubkey, account.identity_pubkey);
    assert_eq!(joined.account_dek, account.account_dek);
    assert_eq!(relay.state.db.machines_for(&account.identity_pubkey).await.unwrap().len(), 3);
    third.relay.token(&relay.url, &joined.machine().unwrap()).await.unwrap();
    for _ in 0..50 {
        if lorca::pairing::status(&second, &nonce)["state"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(lorca::pairing::status(&second, &nonce)["device"]["name"], "Third");

    lorca::sync::unpair_device(&first, &second.this_device_id().unwrap()).await.unwrap();
    let error = lorca::pairing::start(second.clone()).await.unwrap_err();
    assert!(error.to_string().contains("unpaired"), "{error}");
}

/// `POST /v1/machines` takes a paired machine's bearer and new keys, refuses a key paired with
/// other keys, and answers `410` for a revoked one.
#[tokio::test]
async fn attesting_a_machine_takes_a_bearer_and_keeps_keys_apart() {
    let relay = Relay::start(0).await;
    let key = |seed: u8| b64url_encode(ed25519_dalek::SigningKey::from_bytes(&[seed; 32]).verifying_key().as_bytes());
    let (laptop, box_key) = (key(1), b64url_encode(&[2; 32]));
    relay.client.attest(&relay.url, &relay.token, &laptop, &box_key).await.unwrap();
    relay.client.attest(&relay.url, &relay.token, &laptop, &box_key).await.unwrap();
    assert_eq!(relay.state.db.machines_for("identity").await.unwrap().len(), 2);

    let refused = relay.client.attest(&relay.url, &relay.token, &laptop, &b64url_encode(&[3; 32])).await.unwrap_err();
    assert_eq!(refused.status, Some(409));
    let refused = relay.client.attest(&relay.url, "not-a-token", &key(4), &box_key).await.unwrap_err();
    assert_eq!(refused.status, Some(401));
    let refused = relay.client.attest(&relay.url, &relay.token, "not-a-key", &box_key).await.unwrap_err();
    assert_eq!(refused.status, Some(400));

    assert!(relay.state.db.revoke_machine("identity", &laptop).await.unwrap());
    let refused = relay.client.attest(&relay.url, &relay.token, &laptop, &box_key).await.unwrap_err();
    assert_eq!(refused.status, Some(410));
}
