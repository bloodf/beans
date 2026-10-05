//! Local websocket for the app. JSON requests `{ id, method, params }` get `{ id, result }` or
//! `{ id, error }`; events arrive as `{ event, data }`.

use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::app::App;
use crate::events::Event;
use crate::api::dispatch;

#[derive(Debug, Deserialize)]
struct Request {
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

pub async fn serve(app: Arc<App>, ready_stdout: bool) -> anyhow::Result<()> {
    let addr: SocketAddr = ([127, 0, 0, 1], app.config.port).into();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("cannot bind {addr}: {e}. Is another lorca serve running?"))?;
    let port = listener.local_addr()?.port();
    let router = Router::new()
        .route("/", get(index))
        .route("/ws", get(upgrade))
        .with_state((app, port));
    tracing::info!(%addr, "lorca serve");
    if ready_stdout {
        // The parent connects only after this record. Logs go to stderr so stdout is a
        // machine-readable startup channel, independent of tracing filters and formatting.
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{}", json!({ "event": "ready", "port": port }))?;
        stdout.flush()?;
    }
    axum::serve(listener, router).await?;
    Ok(())
}

async fn index() -> impl IntoResponse {
    "lorca"
}

async fn upgrade(State((app, port)): State<(Arc<App>, u16)>, headers: HeaderMap, ws: WebSocketUpgrade) -> impl IntoResponse {
    if let Err(status) = check_upgrade_headers(&headers, port, std::env::var("LORCA_ALLOWED_ORIGINS").ok().as_deref()) {
        return status.into_response();
    }
    ws.on_upgrade(move |socket| connection(app, socket)).into_response()
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

async fn connection(app: Arc<App>, socket: WebSocket) {
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
                tokio::spawn(async move {
                    let response = match dispatch(&app, &request.method, request.params).await {
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
    // The app that said what it was watching is gone.
    app.set_watched_chat(None);
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
        check_upgrade_headers(&headers, 4865, allow)
    }

    #[test]
    fn native_clients_without_origin_need_correct_loopback_host_and_port() {
        assert_eq!(request("127.0.0.1:4865", None, None), Ok(()));
        assert_eq!(request("localhost:4865", None, None), Ok(()));
        for host in ["localhost:4862", "127.0.0.1:4864", "evil.example:4865", "localhost.evil.example:4865", "127.0.0.1:4865.evil.example", "[::1]:4865"] {
            assert_eq!(request(host, None, None), Err(StatusCode::FORBIDDEN), "{host}");
        }
        let headers = HeaderMap::new();
        assert_eq!(check_upgrade_headers(&headers, 4865, None), Err(StatusCode::FORBIDDEN));
    }

    #[test]
    fn browser_same_origin_or_explicit_dev_origin_only() {
        assert_eq!(request("localhost:4865", Some("http://localhost:4865"), None), Ok(()));
        assert_eq!(request("127.0.0.1:4865", Some("http://127.0.0.1:4865"), None), Ok(()));
        assert_eq!(request("127.0.0.1:4865", Some("http://localhost:5178"), Some("http://localhost:5178,http://127.0.0.1:5178")), Ok(()));
        assert_eq!(request("127.0.0.1:4865", Some("http://127.0.0.1:5178"), Some("http://localhost:5178,http://127.0.0.1:5178")), Ok(()));
        assert_eq!(request("127.0.0.1:4865", Some("http://localhost:5178"), None), Err(StatusCode::FORBIDDEN));
        assert_eq!(request("127.0.0.1:4865", Some("https://foreign.example"), Some("http://localhost:5178")), Err(StatusCode::FORBIDDEN));
    }

    #[test]
    fn malformed_and_cross_site_headers_never_upgrade() {
        for origin in ["null", "http://localhost:5178.evil.example", "http://localhost:5178@evil.example", "http://localhost:5178/path", "http://localhost:5178/", "http://localhost:5178?x=1", "http://localhost:5178, http://evil.example", "http://localhost:51780", "https://localhost:5178"] {
            assert_eq!(request("localhost:4865", Some(origin), Some("http://localhost:5178")), Err(StatusCode::FORBIDDEN), "{origin}");
        }
        for allow in ["http://localhost:5178/", "http://localhost:5178/path", "http://localhost:5178@evil.example", "http://localhost:5178,https://evil.example/path", "*"] {
            assert_eq!(request("localhost:4865", Some("http://localhost:5178"), Some(allow)), Err(StatusCode::FORBIDDEN), "{allow}");
        }
        let mut headers = HeaderMap::new();
        headers.append(header::HOST, HeaderValue::from_static("localhost:4865"));
        headers.append(header::HOST, HeaderValue::from_static("evil.example:4865"));
        assert_eq!(check_upgrade_headers(&headers, 4865, None), Err(StatusCode::FORBIDDEN));
        headers.clear();
        headers.insert(header::HOST, HeaderValue::from_static("localhost:4865"));
        headers.append(header::ORIGIN, HeaderValue::from_static("http://localhost:4865"));
        headers.append(header::ORIGIN, HeaderValue::from_static("http://evil.example"));
        assert_eq!(check_upgrade_headers(&headers, 4865, None), Err(StatusCode::FORBIDDEN));
    }
}
