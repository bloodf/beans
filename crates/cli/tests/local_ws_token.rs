#![cfg(feature = "cli")]
use std::process::Stdio;
use std::time::Duration;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::time::timeout;

#[tokio::test]
async fn generated_token_is_required_in_addition_to_host_and_origin() {
    let home = std::env::temp_dir().join(format!("beans-ws-token-{}", uuid::Uuid::new_v4()));
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_beans"));
    command.env_clear().env("RUST_LOG", "off").env("HOME", &home).env("USERPROFILE", &home)
        .env("BEANS_MODELS_FETCH", "0").env("BEANS_MARKETPLACE_FETCH", "0");
    if let Some(root) = std::env::var_os("SystemRoot") { command.env("SystemRoot", root); }
    let mut child = command.args(["serve", "--port", "0", "--ready-stdout", "--home"]).arg(&home)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).spawn().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    timeout(Duration::from_secs(30), stdout.read_line(&mut line)).await.unwrap().unwrap();
    let port = serde_json::from_str::<Value>(&line).unwrap()["port"].as_u64().unwrap() as u16;
    let token = std::fs::read_to_string(home.join("serve-token")).unwrap();
    for (host, origin, credential, expected) in [
        (format!("127.0.0.1:{port}"), None, Some(format!("Bearer {token}")), 101),
        (format!("localhost:{port}"), None, Some(format!("Bearer {token}")), 101),
        (format!("127.0.0.1:{port}"), None, None, 401),
        (format!("127.0.0.1:{port}"), None, Some("Bearer wrong".into()), 401),
        (format!("127.0.0.1:{port}"), Some("https://foreign.invalid"), Some(format!("Bearer {token}")), 403),
        (format!("foreign.invalid:{port}"), None, Some(format!("Bearer {token}")), 403),
    ] {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut request = format!("GET /ws HTTP/1.1\r\nHost: {host}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n");
        if let Some(origin) = origin { request.push_str(&format!("Origin: {origin}\r\n")); }
        if let Some(credential) = credential { request.push_str(&format!("Authorization: {credential}\r\n")); }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        timeout(Duration::from_secs(2), BufReader::new(stream).read_line(&mut response)).await.unwrap().unwrap();
        assert_eq!(response.split_whitespace().nth(1).unwrap().parse::<u16>().unwrap(), expected);
    }
    child.kill().await.unwrap();
    child.wait().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}
