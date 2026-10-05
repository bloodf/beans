//! Public relay feeds. Never reuse the authenticated relay or provider HTTP client.

use std::time::Duration;

use reqwest::header::{ETAG, IF_NONE_MATCH, USER_AGENT};
use reqwest::StatusCode;

const MAX_BYTES: usize = 1024 * 1024;

/// Fetch a bounded JSON feed. `None` means a conditional request got 304.
/// Redirects are refused so neither the requested URL nor its validator leaks to another host.
pub async fn fetch(url: &str, etag: Option<&str>) -> Result<Option<(Option<String>, String)>, String> {
    let parsed = reqwest::Url::parse(url).map_err(|error| error.to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.username() != "" || parsed.password().is_some() {
        return Err("Invalid public feed URL".into());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build().map_err(|error| error.to_string())?;
    let mut request = client.get(parsed).header(USER_AGENT, concat!("lorca/", env!("CARGO_PKG_VERSION")));
    if let Some(etag) = etag {
        request = request.header(IF_NONE_MATCH, etag);
    }
    let mut response = request.send().await.map_err(|error| error.to_string())?;
    if response.status() == StatusCode::NOT_MODIFIED && etag.is_some() {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!("Public feed answered {}", response.status()));
    }
    if response.content_length().is_some_and(|length| length > MAX_BYTES as u64) {
        return Err("Public feed exceeds 1 MiB".into());
    }
    let validator = response.headers().get(ETAG).and_then(|value| value.to_str().ok()).map(str::to_owned);
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if chunk.len() > MAX_BYTES - body.len() {
            return Err("Public feed exceeds 1 MiB".into());
        }
        body.extend_from_slice(&chunk);
    }
    let text = String::from_utf8(body).map_err(|error| error.to_string())?;
    Ok(Some((validator, text)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[tokio::test]
    async fn refuses_oversized_chunked_feed_without_content_length() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/feed", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = socket.read(&mut request);
            socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
            let part = vec![b'a'; 64 * 1024];
            for _ in 0..17 {
                if socket.write_all(b"10000\r\n").is_err() || socket.write_all(&part).is_err() || socket.write_all(b"\r\n").is_err() { break; }
            }
        });
        assert!(fetch(&url, None).await.unwrap_err().contains("1 MiB"));
    }

    #[tokio::test]
    async fn conditional_request_sends_validator_without_credentials() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/feed", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 1024];
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                let count = socket.read(&mut chunk).unwrap();
                if count == 0 { break; }
                request.extend_from_slice(&chunk[..count]);
            }
            let text = String::from_utf8(request).unwrap().to_ascii_lowercase();
            assert!(text.contains("if-none-match: \"v1\""));
            assert!(!text.contains("authorization:"));
            assert!(!text.contains("cookie:"));
            socket.write_all(b"HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\n\r\n").unwrap();
        });
        assert_eq!(fetch(&url, Some("\"v1\"")).await.unwrap(), None);
        server.join().unwrap();
    }
}
