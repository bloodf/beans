//! Memory REST transport: existing certificate trust, no redirects/retries, bounded I/O.
use crate::memory_service::types::{Connection, MemoryError, MemoryScope};
use reqwest::{
    header::{HeaderMap, HeaderValue},
    Method, Url,
};
use serde_json::{Map, Value};
use std::{future::Future, time::Duration};
use tokio_util::sync::CancellationToken;

pub(super) const BODY_LIMIT: usize = 65_536;
pub(super) const SCHEMA_LIMIT: usize = 2_097_152;
pub(super) const DEADLINE: Duration = Duration::from_secs(5);

pub(super) struct MemoryHttp {
    client: reqwest::Client,
    base: Url,
    headers: HeaderMap,
    secrets: Vec<String>,
}
impl MemoryHttp {
    pub(super) fn new(connection: &Connection, headers: HeaderMap) -> Result<Self, MemoryError> {
        let base = validated_endpoint(connection)?;
        let client = lorca_tls::client_builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(DEADLINE)
            .timeout(DEADLINE)
            .build()
            .map_err(|_| error("transport_unavailable"))?;
        let mut secrets: Vec<String> = connection.secret.iter().cloned().collect();
        for name in ["authorization", "x-api-key"] {
            if let Some(value) = headers.get(name).and_then(|h| h.to_str().ok()) {
                let secret = value.strip_prefix("Bearer ").unwrap_or(value);
                if !secrets.iter().any(|stored| stored == secret) {
                    secrets.push(secret.to_owned());
                }
            }
        }
        Ok(Self {
            client,
            base,
            headers,
            secrets,
        })
    }
    pub(super) async fn request(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
        max_bytes: usize,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Value, MemoryError> {
        if max_bytes == 0 || max_bytes > SCHEMA_LIMIT || timeout.is_zero() || timeout > DEADLINE {
            return Err(error("invalid_budget"));
        }
        let mut url = self.base.clone();
        {
            let mut parts = url
                .path_segments_mut()
                .map_err(|_| error("invalid_endpoint"))?;
            parts.pop_if_empty();
            for segment in path.trim_start_matches('/').split('/') {
                identifier(segment)?;
                parts.push(segment);
            }
        }
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        let mut request = self
            .client
            .request(method, url)
            .headers(self.headers.clone());
        if let Some(body) = body {
            let mut bytes = RequestBody(Vec::new());
            serde_json::to_writer(&mut bytes, &body).map_err(|_| error("request_too_large"))?;
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(bytes.0);
        }
        let work = async {
            let mut response = request
                .send()
                .await
                .map_err(|_| error("service_unreachable"))?;
            let status = response.status();
            if !status.is_success() {
                return Err(error(match status.as_u16() {
                    301..=399 => "redirect_refused",
                    401 | 403 => "authorization_failed",
                    404 => "service_not_found",
                    409 => "service_conflict",
                    429 => "service_rate_limited",
                    _ => "service_failed",
                }));
            }
            if response
                .content_length()
                .is_some_and(|n| n > max_bytes as u64)
            {
                return Err(error("response_too_large"));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| error("response_interrupted"))?
            {
                if chunk.len() > max_bytes.saturating_sub(bytes.len()) {
                    return Err(error("response_too_large"));
                }
                bytes.extend_from_slice(&chunk);
            }
            if bytes.is_empty() {
                return Ok(Value::Null);
            }
            serde_json::from_slice(&bytes).map_err(|_| error("invalid_response"))
        };
        bounded(work, timeout, cancel).await
    }
    pub(super) fn public_data(&self, mut value: Value) -> Value {
        fn sanitize(value: &mut Value, secrets: &[String]) {
            match value {
                Value::Object(map) => {
                    map.retain(|key, _| {
                        ![
                            "secret",
                            "password",
                            "credential",
                            "api_key",
                            "authorization",
                            "error_message",
                            "task_payload",
                            "result_metadata",
                        ]
                        .iter()
                        .any(|part| {
                            key.as_bytes()
                                .windows(part.len())
                                .any(|window| window.eq_ignore_ascii_case(part.as_bytes()))
                        })
                    });
                    for value in map.values_mut() {
                        sanitize(value, secrets);
                    }
                }
                Value::Array(items) => {
                    for item in items {
                        sanitize(item, secrets);
                    }
                }
                Value::String(text) => {
                    if secrets
                        .iter()
                        .any(|secret| !secret.is_empty() && text.contains(secret))
                    {
                        *text = "[redacted]".into();
                    } else {
                        *text = crate::memory::scrub(text);
                    }
                }
                _ => {}
            }
        }
        sanitize(&mut value, &self.secrets);
        value
    }
}
pub(super) fn validated_endpoint(connection: &Connection) -> Result<Url, MemoryError> {
    let endpoint = connection
        .endpoint
        .as_deref()
        .ok_or_else(|| error("invalid_endpoint"))?;
    let mut base = Url::parse(endpoint).map_err(|_| error("invalid_endpoint"))?;
    if base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
        || !(base.scheme() == "https"
            || (base.scheme() == "http" && connection.allow_insecure_http))
        || base.path().contains('%')
        || base.path().contains("//")
    {
        return Err(error("invalid_endpoint"));
    }
    let path = base.path().trim_end_matches('/').to_owned();
    base.set_path(&path);
    Ok(base)
}
struct RequestBody(Vec<u8>);
impl std::io::Write for RequestBody {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > BODY_LIMIT.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("memory request exceeds byte limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn error(code: &str) -> MemoryError {
    MemoryError::new(code)
}
pub(super) async fn bounded<T>(
    work: impl Future<Output = Result<T, MemoryError>>,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<T, MemoryError> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(error("cancelled")),
        result = tokio::time::timeout(timeout, work) => result.map_err(|_| error("service_timeout"))?,
    }
}
pub(super) fn header(value: &str) -> Result<HeaderValue, MemoryError> {
    let mut header = HeaderValue::from_str(value).map_err(|_| error("invalid_secret"))?;
    header.set_sensitive(true);
    Ok(header)
}
pub(super) fn identifier(value: &str) -> Result<(), MemoryError> {
    if value.is_empty()
        || value.len() > 256
        || value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
    {
        return Err(error("invalid_identifier"));
    }
    Ok(())
}
pub(super) fn bind(scope: &MemoryScope) -> Result<(), MemoryError> {
    if scope.namespace.len() != 64
        || !scope
            .namespace
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error("invalid_namespace"));
    }
    identifier(&scope.connection_id)
}
pub(super) fn check_scope(bound: &MemoryScope, scope: &MemoryScope) -> Result<(), MemoryError> {
    if bound != scope {
        return Err(error("scope_mismatch"));
    }
    Ok(())
}
pub(super) fn object(value: &Value) -> Result<&Map<String, Value>, MemoryError> {
    value.as_object().ok_or_else(|| error("invalid_request"))
}
pub(super) fn fields(value: &Value, allowed: &[&str]) -> Result<(), MemoryError> {
    if object(value)?
        .keys()
        .any(|key| !allowed.contains(&key.as_str()))
    {
        return Err(error("invalid_request"));
    }
    Ok(())
}
pub(super) fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, MemoryError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| error("invalid_response"))
}
pub(super) fn route(schema: &Value, path: &str, method: &str) -> bool {
    schema
        .get("paths")
        .and_then(|v| v.get(path))
        .and_then(|v| v.get(method))
        .is_some_and(|op| {
            op.is_object() && op.get("deprecated").and_then(Value::as_bool) != Some(true)
        })
}
pub(super) fn method_name(method: &Method) -> &'static str {
    match method.as_str() {
        "GET" => "get",
        "POST" => "post",
        "PATCH" => "patch",
        "PUT" => "put",
        "DELETE" => "delete",
        _ => "",
    }
}
pub(super) fn request_property(schema: &Value, path: &str, method: &str, property: &str) -> bool {
    let Some(mut node) = schema
        .get("paths")
        .and_then(|v| v.get(path))
        .and_then(|v| v.get(method))
        .and_then(|v| v.pointer("/requestBody/content/application~1json/schema"))
    else {
        return false;
    };
    if let Some(reference) = node
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix('#'))
    {
        let Some(resolved) = schema.pointer(reference) else {
            return false;
        };
        node = resolved;
    }
    node.get("properties")
        .and_then(|p| p.get(property))
        .is_some()
}

#[cfg(test)]
pub(super) mod test_server {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        task::JoinHandle,
    };
    pub struct Reply {
        pub status: u16,
        pub body: String,
        pub headers: Vec<(String, String)>,
        pub delay: Duration,
        pub chunked: bool,
    }
    impl Reply {
        pub fn json(value: Value) -> Self {
            Self {
                status: 200,
                body: value.to_string(),
                headers: vec![],
                delay: Duration::ZERO,
                chunked: false,
            }
        }
    }
    pub struct Seen {
        pub method: String,
        pub path: String,
        pub headers: String,
        pub body: Value,
    }
    pub struct Server {
        pub url: String,
        task: Option<JoinHandle<Vec<Seen>>>,
    }
    impl Server {
        pub async fn finish(mut self) -> Vec<Seen> {
            tokio::time::timeout(Duration::from_secs(10), self.task.take().unwrap())
                .await
                .unwrap()
                .unwrap()
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            if let Some(task) = &self.task {
                task.abort();
            }
        }
    }
    pub async fn serve(replies: Vec<Reply>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut seen = vec![];
            for reply in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                let header_end = loop {
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(index) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                let len = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while bytes.len() < header_end + len {
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                }
                let mut first = headers.lines().next().unwrap().split_whitespace();
                seen.push(Seen {
                    method: first.next().unwrap().into(),
                    path: first.next().unwrap().into(),
                    headers: headers.clone(),
                    body: if len == 0 {
                        Value::Null
                    } else {
                        serde_json::from_slice(&bytes[header_end..header_end + len]).unwrap()
                    },
                });
                let mut response = format!("HTTP/1.1 {} Fixture\r\nConnection: close\r\nContent-Type: application/json\r\n",reply.status);
                for (k, v) in reply.headers {
                    response.push_str(&format!("{k}: {v}\r\n"));
                }
                if reply.chunked {
                    response.push_str("Transfer-Encoding: chunked\r\n\r\n");
                } else {
                    response.push_str(&format!("Content-Length: {}\r\n\r\n", reply.body.len()));
                }
                let _ = socket.write_all(response.as_bytes()).await;
                tokio::time::sleep(reply.delay).await;
                if reply.chunked {
                    let _ = socket
                        .write_all(
                            format!("{:x}\r\n{}\r\n0\r\n\r\n", reply.body.len(), reply.body)
                                .as_bytes(),
                        )
                        .await;
                } else {
                    let _ = socket.write_all(reply.body.as_bytes()).await;
                }
            }
            seen
        });
        Server {
            url,
            task: Some(task),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_service::types::BackendKind;
    use serde_json::json;
    use test_server::*;
    fn connection(endpoint: String) -> Connection {
        Connection {
            backend: BackendKind::Hindsight,
            name: "fixture".into(),
            endpoint: Some(endpoint),
            secret: Some("fixture-secret".into()),
            embedding_profile: None,
            options: Some(crate::memory_service::types::BackendOptions::Hindsight {}),
            allow_insecure_http: true,
            extra: Default::default(),
        }
    }
    #[tokio::test]
    async fn redirects_and_chunked_overflow_are_refused_without_secret_errors() {
        let mut redirect = Reply::json(json!({"secret":"fixture-secret"}));
        redirect.status = 302;
        redirect
            .headers
            .push(("Location".into(), "http://127.0.0.1:9/stolen".into()));
        let mut huge = Reply::json(json!("x".repeat(500)));
        huge.chunked = true;
        let server = serve(vec![redirect, huge]).await;
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", header("Bearer fixture-secret").unwrap());
        let http = MemoryHttp::new(&connection(server.url.clone()), headers).unwrap();
        for expected in ["redirect_refused", "response_too_large"] {
            let error = http
                .request(
                    Method::GET,
                    "/health",
                    &[],
                    None,
                    100,
                    DEADLINE,
                    &CancellationToken::new(),
                )
                .await
                .unwrap_err();
            assert_eq!(error.code, expected);
            assert!(!error.message.contains("fixture-secret"));
        }
        assert_eq!(server.finish().await.len(), 2);
    }
    #[tokio::test]
    async fn cancellation_and_deadline_cover_streamed_body() {
        let mut delayed = Reply::json(json!({"ok":true}));
        delayed.delay = Duration::from_millis(200);
        let server = serve(vec![delayed]).await;
        let http = MemoryHttp::new(&connection(server.url.clone()), HeaderMap::new()).unwrap();
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            stop.cancel();
        });
        assert_eq!(
            http.request(Method::GET, "/health", &[], None, 100, DEADLINE, &cancel)
                .await
                .unwrap_err()
                .code,
            "cancelled"
        );
        server.finish().await;
        let mut delayed = Reply::json(json!({"ok":true}));
        delayed.delay = Duration::from_millis(200);
        let server = serve(vec![delayed]).await;
        let http = MemoryHttp::new(&connection(server.url.clone()), HeaderMap::new()).unwrap();
        assert_eq!(
            http.request(
                Method::GET,
                "/health",
                &[],
                None,
                100,
                Duration::from_millis(30),
                &CancellationToken::new()
            )
            .await
            .unwrap_err()
            .code,
            "service_timeout"
        );
        server.finish().await;
    }
    #[tokio::test]
    async fn oversized_outbound_payload_is_rejected_before_sending() {
        let http =
            MemoryHttp::new(&connection("http://127.0.0.1:9".into()), HeaderMap::new()).unwrap();
        let error = http
            .request(
                Method::POST,
                "/memories",
                &[],
                Some(json!({"text":"x".repeat(BODY_LIMIT)})),
                BODY_LIMIT,
                DEADLINE,
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "request_too_large");
    }
    #[test]
    fn endpoint_and_identifiers_cannot_inject_credentials_or_paths() {
        for endpoint in [
            "https://u:p@host",
            "https://host?q=key",
            "https://host#key",
            "file:///tmp/a",
            "http://host",
            "https://host/%2f",
        ] {
            let mut conn = connection(endpoint.into());
            conn.allow_insecure_http = false;
            assert!(MemoryHttp::new(&conn, HeaderMap::new()).is_err());
        }
        for id in ["..", "a/b", "%2f", "a?key", ""] {
            assert!(identifier(id).is_err());
        }
    }
}
