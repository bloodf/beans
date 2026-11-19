//! A Hugging Face stand-in for tests: an origin on 127.0.0.1 that answers each file with a
//! redirect to a CDN on `localhost` (another host name, as Hugging Face's CDN is), which
//! serves the bytes. Each file can instead misbehave as a real hub might, and every request is
//! logged, so a test can tell exactly what was downloaded and when.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::hub::Hub;
use crate::embeddings::assets::RedirectHosts;

/// How the hub answers one file.
#[derive(Clone, Debug)]
pub enum Serve {
    /// Redirect to the CDN, which sends the bytes.
    Bytes(Vec<u8>),
    /// Redirect to the CDN, which announces the full length but stops after `n` bytes.
    Truncated(Vec<u8>, usize),
    /// The origin answers this status, with no redirect.
    Status(u16),
    /// The origin redirects to a host the hub does not allow.
    ForeignRedirect,
    /// The origin redirects to itself, forever.
    RedirectLoop,
}

pub struct MockHub {
    pub hub: Hub,
    files: Arc<Mutex<HashMap<String, Serve>>>,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockHub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl MockHub {
    /// Serves `files` by file name, whatever repository and revision the path names.
    pub async fn start(files: impl IntoIterator<Item = (&str, Serve)>) -> MockHub {
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cdn = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (origin_port, cdn_port) = (origin.local_addr().unwrap().port(), cdn.local_addr().unwrap().port());
        let files: Arc<Mutex<HashMap<String, Serve>>> = Arc::new(Mutex::new(files.into_iter().map(|(name, serve)| (name.to_string(), serve)).collect()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (served, logged) = (files.clone(), requests.clone());
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    Ok((stream, _)) = origin.accept() => { tokio::spawn(answer_origin(stream, served.clone(), logged.clone(), origin_port, cdn_port)); }
                    Ok((stream, _)) = cdn.accept() => { tokio::spawn(answer_cdn(stream, served.clone(), logged.clone())); }
                }
            }
        });
        let hub = Hub {
            base: format!("http://127.0.0.1:{origin_port}"),
            allow_insecure_http: true,
            redirects: RedirectHosts { exact: ["localhost".to_string()].into(), parents: Default::default() },
        };
        MockHub { hub, files, requests, task }
    }

    /// Changes how one file is served from now on.
    pub fn serve(&self, name: &str, serve: Serve) {
        self.files.lock().insert(name.to_string(), serve);
    }

    /// Every request so far, `origin <path>` or `cdn <path>`.
    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().clone()
    }
}

/// A request's path, and whether it asks for its first byte only (`Range: bytes=0-0`).
async fn read_request(stream: &mut TcpStream) -> (String, bool) {
    let mut buffer = vec![0u8; 8192];
    let n = stream.read(&mut buffer).await.unwrap_or(0);
    let request = String::from_utf8_lossy(&buffer[..n]).to_string();
    let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
    let first_byte = request.lines().any(|line| line.to_ascii_lowercase().replace(' ', "") == "range:bytes=0-0");
    (path, first_byte)
}

fn name_of(path: &str) -> String {
    path.rsplit('/').next().unwrap_or("").to_string()
}

fn head(status: &str, extra: &str, length: usize) -> Vec<u8> {
    format!("HTTP/1.1 {status}\r\n{extra}content-length: {length}\r\nconnection: close\r\n\r\n").into_bytes()
}

async fn answer_origin(mut stream: TcpStream, files: Arc<Mutex<HashMap<String, Serve>>>, requests: Arc<Mutex<Vec<String>>>, origin: u16, cdn: u16) {
    let (path, _) = read_request(&mut stream).await;
    requests.lock().push(format!("origin {path}"));
    let serve = files.lock().get(&name_of(&path)).cloned();
    let reply = match serve {
        None => head("404 Not Found", "", 0),
        Some(Serve::Status(code)) => head(&format!("{code} Mock"), "", 0),
        Some(Serve::ForeignRedirect) => head("302 Found", &format!("location: http://example.invalid:{cdn}/blobs/{}\r\n", name_of(&path)), 0),
        Some(Serve::RedirectLoop) => head("302 Found", &format!("location: http://localhost:{origin}{path}\r\n"), 0),
        Some(Serve::Bytes(_) | Serve::Truncated(..)) => head("302 Found", &format!("location: http://localhost:{cdn}/blobs/{}\r\n", name_of(&path)), 0),
    };
    let _ = stream.write_all(&reply).await;
}

async fn answer_cdn(mut stream: TcpStream, files: Arc<Mutex<HashMap<String, Serve>>>, requests: Arc<Mutex<Vec<String>>>) {
    let (path, first_byte) = read_request(&mut stream).await;
    requests.lock().push(format!("cdn {path}"));
    let serve = files.lock().get(&name_of(&path)).cloned();
    let reply = match serve {
        // As Hugging Face's CDN does: the first byte, and the whole size in Content-Range.
        Some(Serve::Bytes(body)) if first_byte && !body.is_empty() => {
            [head("206 Partial Content", &format!("content-range: bytes 0-0/{}\r\n", body.len()), 1), body[..1].to_vec()].concat()
        }
        Some(Serve::Bytes(body)) => [head("200 OK", "", body.len()), body].concat(),
        Some(Serve::Truncated(body, sent)) => [head("200 OK", "", body.len()), body[..sent.min(body.len())].to_vec()].concat(),
        // The origin loops back to itself on the CDN's host name; it still never ends.
        Some(Serve::RedirectLoop) => head("302 Found", &format!("location: {path}\r\n"), 0),
        _ => head("404 Not Found", "", 0),
    };
    let _ = stream.write_all(&reply).await;
}
