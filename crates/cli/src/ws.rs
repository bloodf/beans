//! Local websocket for the app. JSON requests `{ id, method, params }` get `{ id, result }` or
//! `{ id, error }`; events arrive as `{ event, data }`.
//!
//! Only local programs get in. A browser lets any page open a websocket to 127.0.0.1 and leaves
//! the check to the server, so every route refuses a request with an `Origin` (native clients
//! send none) or a `Host` other than this port on loopback (a DNS-rebound page's), and `/ws`
//! takes only `Authorization: Bearer <token>`, the token in `serve-token` in the data directory.

use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Request as HttpRequest, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;

use crate::app::App;
use crate::config::Config;
use crate::events::Event;
use crate::api::dispatch;

#[derive(Debug, Deserialize)]
struct Request {
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Clone)]
struct Server {
    app: Arc<App>,
    port: u16,
    token: Arc<str>,
}

pub async fn serve(app: Arc<App>, ready_stdout: bool) -> anyhow::Result<()> {
    let addr: SocketAddr = ([127, 0, 0, 1], app.config.port).into();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("cannot bind {addr}: {e}. Is another beans serve running?"))?;
    let port = listener.local_addr()?.port();
    // Read or made once the port is ours, so a second serve that fails to bind never replaces it.
    let token = token(&app.config)?;
    tracing::info!(%addr, "beans serve");
    if ready_stdout {
        // The parent connects only after this record. Logs go to stderr so stdout is a
        // machine-readable startup channel, independent of tracing filters and formatting.
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{}", json!({ "event": "ready", "port": port }))?;
        stdout.flush()?;
    }
    let server = Server { app, port, token: token.into() };
    let router = Router::new()
        .route("/", get(index))
        .route("/ws", get(upgrade))
        .layer(middleware::from_fn_with_state(server.clone(), local_only))
        .with_state(server);
    axum::serve(listener, router).await?;
    Ok(())
}

/// The token clients send: the one this data directory has, or a new one, kept for later starts.
fn token(config: &Config) -> anyhow::Result<String> {
    if let Some(token) = config.serve_token() {
        return Ok(token);
    }
    let token = crate::keys::b64(&crate::keys::random_32());
    crate::config::write_private(&config.serve_token_path(), token.as_bytes())?;
    Ok(token)
}

async fn local_only(State(server): State<Server>, request: HttpRequest, next: Next) -> Response {
    if let Err(reason) = check_caller(request.headers(), server.port) {
        // Debug: a page can send these as often as it likes.
        tracing::debug!(path = %request.uri().path(), "refused a request: {reason}");
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}

/// What a browser's request carries and a local client's does not.
fn check_caller(headers: &HeaderMap, port: u16) -> Result<(), &'static str> {
    if headers.contains_key(header::ORIGIN) {
        return Err("it has an Origin, as a web page's does");
    }
    let host = headers.get(header::HOST).and_then(|host| host.to_str().ok()).unwrap_or_default();
    if ![format!("127.0.0.1:{port}"), format!("localhost:{port}")].iter().any(|local| host.eq_ignore_ascii_case(local)) {
        return Err("its Host is not this port on loopback");
    }
    Ok(())
}

fn check_token(headers: &HeaderMap, token: &str) -> bool {
    let sent = headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok()).and_then(|value| value.strip_prefix("Bearer ")).unwrap_or_default();
    sent.as_bytes().ct_eq(token.as_bytes()).into()
}

async fn index() -> impl IntoResponse {
    "beans"
}

/// Keep this guard before `on_upgrade`: loopback binding alone does not stop DNS rebinding or
/// a foreign page opening the local socket from the user's browser.
fn check_upgrade_headers(headers: &HeaderMap, port: u16, allowed_origins: Option<&str>) -> Result<(), StatusCode> {
    let single = |name| {
        let mut values = headers.get_all(name).iter();
        let value = values.next()?.to_str().ok()?;
        values.next().is_none().then_some(value)
    };
    let host = single(header::HOST).ok_or(StatusCode::FORBIDDEN)?;
    if host != format!("localhost:{port}") && host != format!("127.0.0.1:{port}") {
        return Err(StatusCode::FORBIDDEN);
    }

    // Native WebSocket clients omit Origin. A browser must supply an exact origin, not a URL
    // prefix, suffix, wildcard, path, or comma-joined Origin header.
    if !headers.contains_key(header::ORIGIN) {
        return Ok(());
    }
    let origin = single(header::ORIGIN).ok_or(StatusCode::FORBIDDEN)?;
    let origin = exact_origin(origin).ok_or(StatusCode::FORBIDDEN)?;
    if origin == format!("http://{host}") {
        return Ok(());
    }
    let mut matched = false;
    for entry in allowed_origins.unwrap_or_default().split(',').map(str::trim) {
        if exact_origin(entry).is_none() {
            return Err(StatusCode::FORBIDDEN);
        }
        matched |= entry == origin;
    }
    if matched { Ok(()) } else { Err(StatusCode::FORBIDDEN) }
}

fn exact_origin(value: &str) -> Option<&str> {
    let url = reqwest::Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return None;
    }
    (url.origin().ascii_serialization() == value).then_some(value)
}

async fn upgrade(State(server): State<Server>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if let Err(status) = check_upgrade_headers(&headers, server.port, None) {
        return status.into_response();
    }
    if !check_token(&headers, &server.token) {
        tracing::warn!("refused a websocket without this data directory's token");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(move |socket| connection(server.app, socket))
}

/// Numbers the local connections, so what one said of itself goes when it closes.
static CONNECTIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

async fn connection(app: Arc<App>, socket: WebSocket) {
    let connection = CONNECTIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Cancelled as the connection closes, for the requests on it still under way.
    let closed = tokio_util::sync::CancellationToken::new();
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::channel::<String>(256);
    let mut events = app.events.subscribe();

    let writer = tokio::spawn(async move {
        while let Some(text) = out_rx.recv().await {
            if sink.send(WsMessage::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    loop {
        tokio::select! {
            incoming = stream.next() => {
                let Some(Ok(message)) = incoming else { break };
                let text = match message {
                    WsMessage::Text(text) => text.to_string(),
                    WsMessage::Close(_) => break,
                    _ => continue,
                };
                let request: Request = match serde_json::from_str(&text) {
                    Ok(request) => request,
                    Err(error) => {
                        let _ = out_tx.send(json!({ "id": null, "error": { "message": format!("bad request: {error}") } }).to_string()).await;
                        continue;
                    }
                };
                let app = app.clone();
                let out = out_tx.clone();
                let closed = closed.clone();
                tokio::spawn(async move {
                    #[cfg(not(feature = "cli"))]
                    let _ = closed;
                    let result = match request.method.as_str() {
                        // The desktop app on this connection, about its own updates.
                        #[cfg(feature = "cli")]
                        "app.update_status" => crate::app_update::report(&app, connection, &closed, request.params),
                        #[cfg(feature = "cli")]
                        "app.relaunch_when_idle" => crate::app_update::relaunch_when_idle(&app, &closed).await,
                        _ => dispatch(&app, &request.method, request.params).await,
                    };
                    let response = match result {
                        Ok(result) => json!({ "id": request.id, "result": result }),
                        Err(message) => json!({ "id": request.id, "error": { "message": message } }),
                    };
                    let _ = out.send(response.to_string()).await;
                });
            }
            event = events.recv() => {
                match event {
                    Ok(event) => {
                        if let Ok(text) = serde_json::to_string(&event) {
                            if out_tx.send(text).await.is_err() { break; }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let _ = out_tx.send(serde_json::to_string(&Event::Snapshot(app.snapshot())).unwrap_or_default()).await;
                    }
                    Err(_) => break,
                }
            }
        }
    }
    writer.abort();
    closed.cancel();
    // The app that said what it was watching is gone.
    app.watch_chat(None);
    #[cfg(feature = "cli")]
    crate::app_update::disconnected(&app, connection);
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn request(host: &str, origin: Option<&str>, allow: Option<&str>) -> Result<(), StatusCode> {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_str(host).unwrap());
        if let Some(origin) = origin {
            headers.insert(header::ORIGIN, HeaderValue::from_str(origin).unwrap());
        }
        check_upgrade_headers(&headers, 4875, allow)
    }

    #[test]
    fn native_clients_without_origin_need_correct_loopback_host_and_port() {
        assert_eq!(request("127.0.0.1:4875", None, None), Ok(()));
        assert_eq!(request("localhost:4875", None, None), Ok(()));
        for host in ["localhost:4874", "127.0.0.1:4874", "evil.example:4875", "localhost.evil.example:4875", "127.0.0.1:4875.evil.example", "[::1]:4875"] {
            assert_eq!(request(host, None, None), Err(StatusCode::FORBIDDEN), "{host}");
        }
        let headers = HeaderMap::new();
        assert_eq!(check_upgrade_headers(&headers, 4875, None), Err(StatusCode::FORBIDDEN));
    }

    #[test]
    fn browser_same_origin_or_explicit_dev_origin_only() {
        assert_eq!(request("localhost:4875", Some("http://localhost:4875"), None), Ok(()));
        assert_eq!(request("127.0.0.1:4875", Some("http://127.0.0.1:4875"), None), Ok(()));
        assert_eq!(request("127.0.0.1:4875", Some("http://localhost:5178"), Some("http://localhost:5178,http://127.0.0.1:5178")), Ok(()));
        assert_eq!(request("127.0.0.1:4875", Some("http://127.0.0.1:5178"), Some("http://localhost:5178,http://127.0.0.1:5178")), Ok(()));
        assert_eq!(request("127.0.0.1:4875", Some("http://localhost:5178"), None), Err(StatusCode::FORBIDDEN));
        assert_eq!(request("127.0.0.1:4875", Some("https://foreign.example"), Some("http://localhost:5178")), Err(StatusCode::FORBIDDEN));
    }

    #[test]
    fn malformed_and_cross_site_headers_never_upgrade() {
        for origin in ["null", "http://localhost:5178.evil.example", "http://localhost:5178@evil.example", "http://localhost:5178/path", "http://localhost:5178/", "http://localhost:5178?x=1", "http://localhost:5178, http://evil.example", "http://localhost:51780", "https://localhost:5178"] {
            assert_eq!(request("localhost:4875", Some(origin), Some("http://localhost:5178")), Err(StatusCode::FORBIDDEN), "{origin}");
        }
        for allow in ["http://localhost:5178/", "http://localhost:5178/path", "http://localhost:5178@evil.example", "http://localhost:5178,https://evil.example/path", "*"] {
            assert_eq!(request("localhost:4875", Some("http://localhost:5178"), Some(allow)), Err(StatusCode::FORBIDDEN), "{allow}");
        }
        let mut headers = HeaderMap::new();
        headers.append(header::HOST, HeaderValue::from_static("localhost:4875"));
        headers.append(header::HOST, HeaderValue::from_static("evil.example:4875"));
        assert_eq!(check_upgrade_headers(&headers, 4875, None), Err(StatusCode::FORBIDDEN));
        headers.clear();
        headers.insert(header::HOST, HeaderValue::from_static("localhost:4875"));
        headers.append(header::ORIGIN, HeaderValue::from_static("http://localhost:4875"));
        headers.append(header::ORIGIN, HeaderValue::from_static("http://evil.example"));
        assert_eq!(check_upgrade_headers(&headers, 4875, None), Err(StatusCode::FORBIDDEN));
    }
}
