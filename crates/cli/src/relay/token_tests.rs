use super::*;
use axum::{extract::State, routing::{get, post}, Json, Router};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;

#[derive(Default)]
struct AuthState {
    calls: AtomicUsize,
    // Only the first verification waits; later requests can complete while it is suspended.
    pause_first: bool,
    entered: Notify,
    release: Notify,
}

struct AuthServer {
    url: String,
    state: Arc<AuthState>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for AuthServer {
    fn drop(&mut self) { self.task.abort(); }
}

impl AuthServer {
    async fn start(pause_first: bool) -> Self {
        let state = Arc::new(AuthState { pause_first, ..Default::default() });
        let router = Router::new()
            .route("/v1/health", get(|| async { Json(json!({
                "ok": true, "service": "beans-relay", "format": "beans-v2",
                "protocol": PROTOCOL, "min_protocol": PROTOCOL,
                "min_roster_protocol": MIN_ROSTER_PROTOCOL, "memory_config_version": 1
            })) }))
            .route("/v1/auth/challenge", post(|| async { Json(json!({"nonce": "test-challenge"})) }))
            .route("/v1/auth/verify", post(|State(state): State<Arc<AuthState>>, Json(body): Json<Value>| async move {
                let call = state.calls.fetch_add(1, Ordering::SeqCst) + 1;
                if state.pause_first && call == 1 {
                    state.entered.notify_one();
                    state.release.notified().await;
                }
                Json(json!({"token": format!("{}:{call}", body["machine_pubkey"].as_str().unwrap()), "expires_at": now_unix() + 3600}))
            }))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
        Self { url, state, task }
    }

    async fn entered(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(5), self.state.entered.notified()).await.unwrap();
    }
}

#[tokio::test]
async fn cached_bearer_is_bound_to_relay_and_machine() {
    let first = AuthServer::start(false).await;
    let second = AuthServer::start(false).await;
    let client = RelayClient::new().unwrap();
    let machine = Machine::generate();
    let other = Machine::generate();
    let a = client.token(&first.url, &machine).await.unwrap();
    assert_eq!(client.token(&first.url, &machine).await.unwrap(), a);
    assert_eq!(first.state.calls.load(Ordering::SeqCst), 1);
    assert_eq!(client.token(&second.url, &machine).await.unwrap(), a);
    assert_eq!(second.state.calls.load(Ordering::SeqCst), 1, "new relay requires authentication even for the same Device");
    let b = client.token(&second.url, &other).await.unwrap();
    assert_ne!(a, b);
    assert_eq!(second.state.calls.load(Ordering::SeqCst), 2);
    assert_eq!(client.token(&second.url, &other).await.unwrap(), b);
    assert_eq!(second.state.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn forget_rejects_delayed_authentication_even_for_same_namespace() {
    let server = AuthServer::start(true).await;
    let client = Arc::new(RelayClient::new().unwrap());
    let machine = Arc::new(Machine::generate());
    let pending = {
        let client = client.clone(); let machine = machine.clone(); let url = server.url.clone();
        tokio::spawn(async move { client.authenticate(&url, &machine).await })
    };
    server.entered().await;
    client.forget_token();
    let fresh = client.token(&server.url, &machine).await.unwrap();
    server.state.release.notify_one();
    let error = pending.await.unwrap().unwrap_err();
    assert_eq!(error.message, "relay authentication context changed");
    assert_eq!(client.token(&server.url, &machine).await.unwrap(), fresh);
    assert_eq!(server.state.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn namespace_switch_rejects_delayed_token_completion() {
    let server = AuthServer::start(true).await;
    let client = Arc::new(RelayClient::new().unwrap());
    let old = Arc::new(Machine::generate());
    let pending = {
        let client = client.clone(); let old = old.clone(); let url = server.url.clone();
        tokio::spawn(async move { client.token(&url, &old).await })
    };
    server.entered().await;
    let replacement = Machine::generate();
    let fresh = client.token(&server.url, &replacement).await.unwrap();
    server.state.release.notify_one();
    assert!(pending.await.unwrap().is_err());
    assert_eq!(client.token(&server.url, &replacement).await.unwrap(), fresh);
    assert_eq!(server.state.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn namespace_round_trip_rejects_original_authentication() {
    let server = AuthServer::start(true).await;
    let client = Arc::new(RelayClient::new().unwrap());
    let original = Arc::new(Machine::generate());
    let pending = {
        let client = client.clone(); let original = original.clone(); let url = server.url.clone();
        tokio::spawn(async move { client.token(&url, &original).await })
    };
    server.entered().await;
    client.token(&server.url, &Machine::generate()).await.unwrap();
    let fresh = client.token(&server.url, &original).await.unwrap();
    server.state.release.notify_one();
    assert!(pending.await.unwrap().is_err(), "returning to the original namespace must not revive its old admission");
    assert_eq!(client.token(&server.url, &original).await.unwrap(), fresh);
    assert_eq!(server.state.calls.load(Ordering::SeqCst), 3);
}
