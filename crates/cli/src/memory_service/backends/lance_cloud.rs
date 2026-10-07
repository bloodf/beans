//! The official SDK exposes no redirect/body-limit or system-TLS client hook.
//! A private, token-authenticated loopback bridge enforces those policies without replacing the SDK.
use crate::memory_service::types::MemoryError;
use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{Request, Response, StatusCode},
    Router,
};
use futures::StreamExt;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
const BODY_LIMIT: usize = 8 * 1024 * 1024;
struct Forward {
    client: reqwest::Client,
    target: reqwest::Url,
    key: String,
    token: String,
}
pub(super) struct CloudBridge {
    pub url: String,
    pub token: String,
    stop: CancellationToken,
}
impl Drop for CloudBridge {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
impl CloudBridge {
    pub async fn start(database: &str, region: &str, key: &str) -> Result<Self, MemoryError> {
        let target = reqwest::Url::parse(&format!("https://{database}.{region}.api.lancedb.com/"))
            .map_err(|_| MemoryError::new("invalid_lance_cloud_target"))?;
        Self::start_to(target, key).await
    }
    pub(super) async fn start_to(target: reqwest::Url, key: &str) -> Result<Self, MemoryError> {
        let client = beans_tls::client_builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| MemoryError::new("lance_transport_unavailable"))?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| MemoryError::new("lance_transport_unavailable"))?;
        let address = listener
            .local_addr()
            .map_err(|_| MemoryError::new("lance_transport_unavailable"))?;
        let token = uuid::Uuid::new_v4().to_string();
        let stop = CancellationToken::new();
        let state = Arc::new(Forward {
            client,
            target,
            key: key.into(),
            token: token.clone(),
        });
        let router = Router::new().fallback(forward).with_state(state);
        let cancelled = stop.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(cancelled.cancelled_owned())
                .await;
        });
        Ok(Self {
            url: format!("http://{address}"),
            token,
            stop,
        })
    }
}
async fn forward(State(state): State<Arc<Forward>>, request: Request<Body>) -> Response<Body> {
    if request
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        != Some(state.token.as_str())
    {
        return failure(StatusCode::UNAUTHORIZED);
    }
    let (parts, body) = request.into_parts();
    let path = parts.uri.path();
    if !path.starts_with("/v1/table/beans_memory_v1/") || path.contains("..") || path.contains('%')
    {
        return failure(StatusCode::BAD_REQUEST);
    }
    let body = match to_bytes(body, BODY_LIMIT).await {
        Ok(v) => v,
        Err(_) => return failure(StatusCode::PAYLOAD_TOO_LARGE),
    };
    let relative = parts
        .uri
        .path_and_query()
        .map_or(path, |v| v.as_str())
        .trim_start_matches('/');
    let target = match state.target.join(relative) {
        Ok(v) if v.origin() == state.target.origin() => v,
        _ => return failure(StatusCode::BAD_REQUEST),
    };
    let mut outgoing = state
        .client
        .request(parts.method, target)
        .header("x-api-key", &state.key)
        .body(body);
    for name in [
        "content-type",
        "accept",
        "x-request-id",
        "x-lancedb-database",
        "x-request-timeout-ms",
        "x-lancedb-min-version",
        "x-lancedb-min-timestamp",
        "x-lancedb-min-read-version",
    ] {
        if let Some(value) = parts.headers.get(name) {
            outgoing = outgoing.header(name, value);
        }
    }
    let response = match outgoing.send().await {
        Ok(v) => v,
        Err(_) => return failure(StatusCode::BAD_GATEWAY),
    };
    if response.status().is_redirection() {
        return failure(StatusCode::BAD_GATEWAY);
    }
    if !response.status().is_success() {
        return failure(response.status());
    }
    if response
        .content_length()
        .is_some_and(|n| n > BODY_LIMIT as u64)
    {
        return failure(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let status = response.status();
    let headers = response.headers().clone();
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(part) = stream.next().await {
        match part {
            Ok(part) if bytes.len() + part.len() <= BODY_LIMIT => bytes.extend_from_slice(&part),
            _ => return failure(StatusCode::PAYLOAD_TOO_LARGE),
        }
    }
    let mut result = Response::builder().status(status);
    for name in ["content-type", "phalanx-version", "x-lancedb-version"] {
        if let Some(value) = headers.get(name) {
            result = result.header(name, value);
        }
    }
    result
        .body(Body::from(bytes))
        .unwrap_or_else(|_| failure(StatusCode::BAD_GATEWAY))
}
fn failure(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from("{\"error\":\"memory_service_request_failed\"}"))
        .expect("static response")
}
