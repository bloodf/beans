use super::*;
use serde_json::json;

#[test]
fn durindoor_public_catalog_keeps_combos_without_a_kind() {
    let body = json!({"data": [
        {"id":"image-helper","owned_by":"combo","capabilities":{"tools":true}},
        {"id":"audio-review","owned_by":"combo"},
        {"id":"guard","owned_by":"combo"},
        {"id":"web","owned_by":"combo","kind":"webSearch"}
    ]});
    let listed = listed_models(&body).unwrap();
    assert_eq!(listed.into_iter().filter(|(not_chat,_)| !not_chat).map(|(_,m)| m.id).collect::<Vec<_>>(), ["image-helper", "audio-review", "guard"]);
}

#[test]
fn durindoor_aliases_keep_explicit_chat_and_nested_limits() {
    let (not_chat, model) = listed_model(&json!({
        "id": "image-helper", "kind": "llm", "owned_by": "combo",
        "capabilities": { "completion_chat": true, "contextWindow": 200000, "maxOutput": 32000, "vision": true }
    })).unwrap();
    assert!(!not_chat, "explicit chat metadata overrides alias words");
    assert_eq!((model.context_window, model.max_output, model.images), (Some(200000), Some(32000), Some(true)));
    assert!(listed_model(&json!({ "id": "friendly", "kind": "webSearch" })).unwrap().0);
}

#[test]
fn normalizes_custom_roots() {
    assert_eq!(custom_base_url(Some(" https://proxy.example/v1/ ")).unwrap(), Some("https://proxy.example/v1".into()));
    assert_eq!(custom_base_url(Some("  ")).unwrap(), None);
    assert_eq!(opencode_root("https://proxy.example/v1"), "https://proxy.example");
    assert!(custom_base_url(Some("proxy.example")).unwrap_err().contains("http://"));
}

#[test]
fn a_pasted_endpoint_becomes_the_root_its_protocol_extends() {
    let root = |api, url| custom_root(api, url).unwrap();
    assert_eq!(root(CustomApi::ChatCompletions, "https://openrouter.ai/api/v1/"), "https://openrouter.ai/api/v1");
    assert_eq!(root(CustomApi::ChatCompletions, "http://localhost:11434/v1/chat/completions"), "http://localhost:11434/v1");
    assert_eq!(root(CustomApi::Responses, "https://gateway.example/v1/responses"), "https://gateway.example/v1");
    assert_eq!(root(CustomApi::Messages, "https://api.anthropic.com/v1/messages"), "https://api.anthropic.com");
    assert_eq!(root(CustomApi::Messages, "https://api.moonshot.ai/anthropic"), "https://api.moonshot.ai/anthropic");
    assert_eq!(custom_root(CustomApi::Messages, " ").unwrap_err(), "Enter the server's base URL");
}

#[test]
fn kinds_are_slugs_of_the_name() {
    let mut credentials = Credentials::default();
    assert_eq!(custom_kind(&credentials, "OpenRouter"), "custom:openrouter");
    assert_eq!(custom_kind(&credentials, "  My Mac Studio (LM Studio) "), "custom:my-mac-studio-lm-studio");
    assert_eq!(custom_kind(&credentials, "本地模型"), "custom:provider");
    let provider = CustomProvider { name: "OpenRouter".into(), api: CustomApi::ChatCompletions, base_url: String::new(), api_key: String::new(), models: Vec::new(), created_at: 0, integration: None };
    credentials.custom.insert("custom:openrouter".into(), provider);
    assert_eq!(custom_kind(&credentials, "openrouter!"), "custom:openrouter-2");
}

#[test]
fn model_lists_tell_windows_inputs_and_what_is_not_for_chat() {
    // OpenRouter
    let (not_chat, model) = listed_model(&json!({
        "id": "anthropic/claude-sonnet-5", "name": "Anthropic: Claude Sonnet 5", "context_length": 1000000,
        "architecture": { "input_modalities": ["text", "image"], "output_modalities": ["text"] },
        "top_provider": { "max_completion_tokens": 128000 }
    }))
    .unwrap();
    assert!(!not_chat);
    assert_eq!(model.name.as_deref(), Some("Anthropic: Claude Sonnet 5"));
    assert_eq!((model.context_window, model.max_output, model.images), (Some(1_000_000), Some(128_000), Some(true)));
    // Anthropic
    let (_, model) = listed_model(&json!({ "type": "model", "id": "claude-opus-5", "display_name": "Claude Opus 5" })).unwrap();
    assert_eq!((model.name.as_deref(), model.context_window, model.images), (Some("Claude Opus 5"), None, None));
    // Mistral and vLLM
    let (_, model) = listed_model(&json!({ "id": "pixtral-large", "max_context_length": 131072, "capabilities": { "vision": true, "completion_chat": true } })).unwrap();
    assert_eq!((model.context_window, model.images), (Some(131_072), Some(true)));
    assert!(listed_model(&json!({ "id": "mistral-embed", "capabilities": { "completion_chat": false } })).unwrap().0);
    assert_eq!(listed_model(&json!({ "id": "qwen3-32b", "max_model_len": 40960 })).unwrap().1.context_window, Some(40_960));
    assert!(listed_model(&json!({ "id": "black-forest-labs/FLUX.1-schnell", "type": "image" })).unwrap().0);
    assert!(!listed_model(&json!({ "id": "meta-llama/Llama-4-Scout", "type": "chat" })).unwrap().0);
    assert!(listed_model(&json!({ "id": "" })).is_none());
    // A bare array (Together) and Ollama's `models`.
    assert_eq!(listed_models(&json!([{ "id": "a" }, { "id": "b" }])).unwrap().len(), 2);
    assert_eq!(listed_models(&json!({ "models": [{ "name": "qwen3:8b" }] })).unwrap()[0].1.id, "qwen3:8b");
    assert!(listed_models(&json!({ "status": "ok" })).is_none());
}

struct ScratchApp(Arc<App>, std::path::PathBuf);

impl Drop for ScratchApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

fn scratch_app() -> ScratchApp {
    let home = std::env::temp_dir().join(format!("lorca-provider-auth-{}", uuid::Uuid::new_v4()));
    ScratchApp(App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap(), home)
}

/// Answers each request with the next status and body, and hands back the request lines.
fn serve(answers: Vec<(&'static str, String)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        use std::io::{Read, Write};
        let mut seen = Vec::new();
        for (status, body) in answers {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let read = socket.read(&mut request).unwrap();
            seen.push(String::from_utf8_lossy(&request[..read]).to_string());
            let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            socket.write_all(reply.as_bytes()).unwrap();
        }
        seen
    });
    (root, server)
}

fn input(name: &str, api: &str, base_url: &str, models: &[&str]) -> CustomInput {
    CustomInput { name: name.into(), api: api.into(), base_url: base_url.into(), models: models.iter().map(|m| m.to_string()).collect(), ..Default::default() }
}

#[tokio::test]
async fn legacy_discovery_uses_only_models_and_keeps_identity_on_edit() {
    let scratch = scratch_app();
    let list = json!({ "data": [{ "id": "image-helper", "kind": "llm", "capabilities": { "tools": true, "reasoning": false } }] }).to_string();
    let (root, server) = serve(vec![("200 OK", list.clone()), ("200 OK", list)]);
    let mut gateway = input("DurinDoor", "chat-completions", &root, &[]);
    gateway.integration = Some("durindoor".into());
    let kind = connect_custom(&scratch.0, gateway).await.unwrap();
    let mut edited = input("My gateway", "chat-completions", &format!("{root}/v1/models"), &[]);
    edited.kind = Some(kind.clone());
    connect_custom(&scratch.0, edited).await.unwrap();
    let requests = server.join().unwrap();
    assert!(requests.iter().all(|request| request.starts_with("GET /v1/models ")));
    assert!(!requests.iter().any(|r| r.to_ascii_lowercase().contains("authorization:")));
    let credentials = scratch.0.credentials.lock().unwrap();
    assert_eq!(credentials.custom[&kind].integration, Some(CustomIntegration::Durindoor));
    assert_eq!(credentials.custom[&kind].models[0].id, "image-helper");
    assert!(credentials.statuses().last().unwrap().models[0].levels.is_empty());
}

#[tokio::test]
async fn discovery_rejects_malformed_and_oversized_bodies() {
    let scratch = scratch_app();
    for body in ["not json".into(), " ".repeat(MODEL_RESPONSE_LIMIT + 1)] {
        let (root, server) = serve(vec![("200 OK", body)]);
        assert!(list_custom_models(&scratch.0, "Legacy", "chat-completions", &root, "", Some("durindoor")).await.is_err());
        server.join().unwrap();
    }
    let (root, server) = serve(vec![("200 OK", " ".repeat(MODEL_RESPONSE_LIMIT + 1))]);
    assert!(list_custom_models(&scratch.0, "Generic", "chat-completions", &root, "", None).await.unwrap_err().contains("too large"));
    server.join().unwrap();
}

#[tokio::test]
async fn credential_redirects_are_not_followed() {
    use std::io::{Read, Write};
    let scratch = scratch_app();
    let sink = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    sink.set_nonblocking(true).unwrap();
    let destination = format!("http://{}/capture", sink.local_addr().unwrap());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        socket.read(&mut request).unwrap();
        write!(socket, "HTTP/1.1 307 Temporary Redirect\r\nLocation: {destination}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    assert!(list_custom_models(&scratch.0, "DurinDoor", "chat-completions", &root, "fixture-key-not-real", Some("durindoor")).await.is_err());
    server.join().unwrap();
    assert_eq!(sink.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
}

#[test]
fn durindoor_roots_validate_and_normalize_without_guessing_hosts() {
    let integration = Some(CustomIntegration::Durindoor);
    for path in ["", "/v1/", "/v1/models", "/v1/chat/completions", "/v1/realtime/auth"] {
        assert_eq!(integration_root(CustomApi::ChatCompletions, &format!("http://localhost:20128{path}"), integration).unwrap(), "http://localhost:20128/v1");
    }
    for root in ["http://", "ftp://example.com", "https://user:secret@example.com/v1", "https://example.com/v1?key=secret", "https://example.com/v1#fragment"] {
        assert!(integration_root(CustomApi::ChatCompletions, root, integration).is_err());
    }
    assert!(CustomIntegration::parse(Some("unknown")).is_err());
}

#[tokio::test]
async fn durindoor_rejected_access_never_fetches_public_models_or_saves() {
    let scratch = scratch_app();
    let (root, server) = serve(vec![("401 Unauthorized", "{}".into())]);
    let mut gateway = input("DurinDoor", "chat-completions", &root, &[]);
    gateway.integration = Some("durindoor".into());
    gateway.api_key = "fixture-key-not-real".into();
    assert_eq!(connect_custom(&scratch.0, gateway).await.unwrap_err(), "DurinDoor rejected that key");
    assert!(server.join().unwrap()[0].starts_with("GET /v1/models "));
    assert!(scratch.0.credentials.lock().unwrap().custom.is_empty());
}

#[tokio::test]
async fn a_custom_provider_takes_the_models_its_server_lists() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let list = json!({ "object": "list", "data": [
        { "id": "qwen3:8b", "object": "model" },
        { "id": "nomic-embed-text", "object": "model", "capabilities": { "completion_chat": false } },
        { "id": "llava", "object": "model", "context_window": 8192 }
    ]});
    let (root, server) = serve(vec![("200 OK", list.to_string())]);
    let kind = connect_custom(app, input("Ollama", "chat-completions", &format!("{root}/v1/chat/completions"), &[])).await.unwrap();
    assert_eq!(kind, "custom:ollama");
    let request = &server.join().unwrap()[0];
    assert!(request.starts_with("GET /v1/models "), "{request}");
    assert!(!request.to_ascii_lowercase().contains("authorization"), "no key, no header");

    let credentials = app.credentials.lock().unwrap();
    let provider = &credentials.custom["custom:ollama"];
    assert_eq!(provider.base_url, format!("{root}/v1"));
    assert_eq!(provider.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["qwen3:8b", "llava"]);
    assert_eq!(provider.models[1].context_window, Some(8192));
    let status = credentials.statuses().into_iter().find(|status| status.kind == "custom:ollama").unwrap();
    assert_eq!((status.name.as_deref(), status.api, status.detail.as_str()), (Some("Ollama"), Some(CustomApi::ChatCompletions), provider.base_url.as_str()));
    assert!(credentials.changed_at.contains_key("custom:ollama"));
}

#[tokio::test]
async fn the_users_models_keep_their_order_and_take_what_the_list_says() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let list = json!({ "data": [{ "id": "claude-opus-5", "display_name": "Claude Opus 5" }], "has_more": false });
    let (root, server) = serve(vec![("200 OK", list.to_string())]);
    let mut proxy = input("Claude proxy", "messages", &format!("{root}/v1"), &["claude-sonnet-5", "claude-opus-5", "claude-sonnet-5"]);
    proxy.api_key = " sk-proxy-1234 ".into();
    let kind = connect_custom(app, proxy).await.unwrap();
    let request = &server.join().unwrap()[0];
    assert!(request.starts_with("GET /v1/models?limit=1000 "), "{request}");
    assert!(request.contains("x-api-key: sk-proxy-1234"), "{request}");
    let provider = app.credentials.lock().unwrap().custom[&kind].clone();
    assert_eq!(provider.base_url, root);
    assert_eq!(provider.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["claude-sonnet-5", "claude-opus-5"]);
    assert_eq!(provider.models[1].name.as_deref(), Some("Claude Opus 5"));

    // Saving it again under its kind keeps the kind and when it was added.
    let (root, server) = serve(vec![("404 Not Found", "{}".into())]);
    let mut edited = input("Claude", "messages", &root, &["claude-opus-5"]);
    edited.kind = Some(kind.clone());
    assert_eq!(connect_custom(app, edited).await.unwrap(), kind);
    server.join().unwrap();
    let saved = app.credentials.lock().unwrap().custom[&kind].clone();
    assert_eq!((saved.name.as_str(), saved.created_at, saved.models.len()), ("Claude", provider.created_at, 1));
}

#[tokio::test]
async fn a_custom_provider_needs_a_key_it_takes_and_models_to_offer() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let (root, server) = serve(vec![("401 Unauthorized", "{}".into()), ("401 Unauthorized", "{}".into()), ("404 Not Found", "{}".into())]);
    let mut keyed = input("Gateway", "chat-completions", &root, &["m"]);
    keyed.api_key = "bad".into();
    assert_eq!(connect_custom(app, keyed).await.unwrap_err(), "Gateway rejected that key");
    assert_eq!(connect_custom(app, input("Gateway", "chat-completions", &root, &["m"])).await.unwrap_err(), "Gateway needs an API key");
    assert!(connect_custom(app, input("Gateway", "chat-completions", &root, &[])).await.unwrap_err().contains("Add the model ids yourself"));
    server.join().unwrap();

    assert_eq!(connect_custom(app, input(" ", "chat-completions", &root, &["m"])).await.unwrap_err(), "Name the provider");
    assert_eq!(connect_custom(app, input("Anthropic", "messages", &root, &["m"])).await.unwrap_err(), "A provider named Anthropic exists already");
    assert!(connect_custom(app, input("Lab", "completions", &root, &["m"])).await.unwrap_err().starts_with("Unknown API"));
    assert!(app.credentials.lock().unwrap().custom.is_empty());
}

#[tokio::test]
async fn the_picker_gets_the_chat_models_or_hears_there_is_no_list() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let list = json!({ "data": [{ "id": "gpt-6-sol", "context_window": 1050000 }, { "id": "text-embedding-3-small", "type":"embedding" }] });
    let (root, server) = serve(vec![("200 OK", list.to_string()), ("404 Not Found", "{}".into()), ("401 Unauthorized", "{}".into())]);
    let models = list_custom_models(app, "OpenAI", "responses", &format!("{root}/v1/responses"), " sk-1 ", None).await.unwrap().unwrap();
    assert_eq!(models.iter().map(|m| (m.id.as_str(), m.context_window)).collect::<Vec<_>>(), [("gpt-6-sol", Some(1_050_000))]);
    assert_eq!(list_custom_models(app, "", "chat-completions", &root, "", None).await.unwrap(), None);
    assert_eq!(list_custom_models(app, "", "chat-completions", &root, "bad", None).await.unwrap_err(), "The server rejected that key");
    let requests = server.join().unwrap();
    assert!(requests[0].starts_with("GET /v1/models ") && requests[0].contains("authorization: Bearer sk-1"), "{}", requests[0]);
    assert!(list_custom_models(app, "", "chat-completions", "ftp://lab", "", None).await.unwrap_err().contains("http://"));
    assert!(app.credentials.lock().unwrap().custom.is_empty(), "listing saves nothing");
}

#[tokio::test]
async fn refresh_adds_models_but_preserves_manual_ids_url_key_and_unchanged_stamp() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let list = json!({ "data": [{ "id": "new", "context_window": 128000 }, { "id": "nomic-embed-text", "type":"embedding" }] }).to_string();
    let (root, server) = serve(vec![("404 Not Found", "{}".into()), ("200 OK", list.clone()), ("200 OK", list)]);
    let mut provider = input("Lab", "responses", &root, &["picked"]);
    provider.api_key = "secret".into();
    let kind = connect_custom(app, provider).await.unwrap();
    let before = app.credentials.lock().unwrap().changed_at[&kind];
    assert_eq!(refresh_custom_models(app).await.unwrap(), 1);
    let credentials = app.credentials.lock().unwrap().clone();
    assert_eq!(credentials.custom[&kind].models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["picked", "new"]);
    assert_eq!(credentials.custom[&kind].models[1].context_window, Some(128_000));
    assert_eq!((credentials.custom[&kind].base_url.as_str(), credentials.custom[&kind].api_key.as_str()), (root.as_str(), "secret"));
    assert!(credentials.changed_at[&kind] > before);
    assert_eq!(refresh_custom_models(app).await.unwrap(), 0);
    assert_eq!(app.credentials.lock().unwrap().changed_at[&kind], credentials.changed_at[&kind]);
    assert_eq!(Credentials::load(&app.config).custom[&kind], credentials.custom[&kind]);
    assert_eq!(server.join().unwrap().len(), 3);
}

#[tokio::test]
async fn missing_list_and_rejected_key_leave_catalog_untouched() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let (root, server) = serve(vec![("404 Not Found", "{}".into()), ("404 Not Found", "{}".into()), ("401 Unauthorized", "{}".into())]);
    let kind = connect_custom(app, input("Lab", "responses", &root, &["picked"])).await.unwrap();
    let before = app.credentials.lock().unwrap().changed_at[&kind];
    assert_eq!(refresh_custom_models(app).await.unwrap(), 0);
    assert_eq!(refresh_custom_models(app).await.unwrap_err(), format!("Could not refresh models for {kind}"));
    let credentials = app.credentials.lock().unwrap();
    assert_eq!(credentials.changed_at[&kind], before);
    assert_eq!(credentials.custom[&kind].models[0].id, "picked");
    server.join().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_flight_refresh_cannot_restore_a_deleted_provider() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let kind = {
        let (root, server) = serve(vec![("404 Not Found", "{}".into())]);
        let kind = connect_custom(app, input("Lab", "responses", &root, &["picked"])).await.unwrap();
        server.join().unwrap();
        kind
    };
    app.update_credentials(&kind, |c| c.custom.get_mut(&kind).unwrap().base_url = root.clone()).unwrap();
    let (requested, received) = std::sync::mpsc::channel();
    let (release, proceed) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        use std::io::{Read, Write};
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        socket.read(&mut request).unwrap();
        requested.send(()).unwrap();
        proceed.recv().unwrap();
        let body = json!({ "data": [{ "id": "new" }] }).to_string();
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    let app_for_refresh = app.clone();
    let refresh = tokio::spawn(async move { refresh_custom_models(&app_for_refresh).await });
    received.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    disconnect(app, &kind).unwrap();
    release.send(()).unwrap();
    assert_eq!(refresh.await.unwrap().unwrap(), 0);
    server.join().unwrap();
    assert!(!app.credentials.lock().unwrap().custom.contains_key(&kind));
}

#[tokio::test]
async fn deleting_a_custom_provider_reaches_every_device_as_a_change() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let (root, server) = serve(vec![("404 Not Found", "{}".into())]);
    let kind = connect_custom(app, input("Lab", "responses", &root, &["gpt-oss-120b"])).await.unwrap();
    server.join().unwrap();
    disconnect(app, &kind).unwrap();
    let credentials = app.credentials.lock().unwrap();
    assert!(credentials.custom.is_empty());
    assert!(credentials.changed_at.contains_key(&kind), "the deletion is a change the merge carries");
    drop(credentials);
    assert_eq!(disconnect(app, &kind).unwrap_err(), format!("Unknown provider {kind}"));
}

#[tokio::test]
async fn compatible_discovery_errors_are_not_unsupported_listing() {
    let scratch = scratch_app();
    for (status, body) in [
        ("429 Too Many Requests", "{}".into()),
        ("503 Service Unavailable", "{}".into()),
        ("200 OK", json!({"status":"ok"}).to_string()),
    ] {
        let (root, server) = serve(vec![(status, body)]);
        assert!(connect_custom(&scratch.0, input("Compatible", "responses", &root, &["manual"])).await.is_err());
        server.join().unwrap();
        assert!(scratch.0.credentials.lock().unwrap().custom.is_empty());
    }
}

#[tokio::test]
async fn compatible_messages_discovery_follows_official_cursors() {
    let scratch = scratch_app();
    let (root, server) = serve(vec![
        ("200 OK", json!({"data":[{"id":"claude-a","type":"model","display_name":"A"}],"has_more":true,"first_id":"claude-a","last_id":"claude-a"}).to_string()),
        ("200 OK", json!({"data":[{"id":"audio-helper","type":"model"}],"has_more":false,"first_id":"audio-helper","last_id":"audio-helper"}).to_string()),
    ]);
    let found = list_custom_models(&scratch.0, "Compatible", "messages", &format!("{root}/v1/messages"), "fixture-key", None).await.unwrap().unwrap();
    assert_eq!(found.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["claude-a", "audio-helper"]);
    let requests = server.join().unwrap();
    assert!(requests[0].starts_with("GET /v1/models?limit=1000 "));
    assert!(requests[1].starts_with("GET /v1/models?limit=1000&after_id=claude-a "));
    for request in requests {
        assert!(request.contains("x-api-key: fixture-key"));
        assert!(request.contains("anthropic-version: 2023-06-01"));
        assert!(!request.contains("authorization:"));
    }
}

#[test]
fn compatible_catalog_preserves_callable_ids_without_word_guesses() {
    let models = listed_models(&json!({"data":[
        {"id":"image-helper"}, {"id":"audio-review"}, {"id":"guard"},
        {"id":"combo","kind":"combo"}, {"id":"future","kind":"future-kind"},
        {"id":"opaque","capabilities":{"completion_chat":false}},
        {"id":"embed","type":"embedding"}
    ]})).unwrap();
    assert_eq!(models.into_iter().filter(|(not_chat, _)| !not_chat).map(|(_, m)| m.id).collect::<Vec<_>>(), ["image-helper", "audio-review", "guard", "combo", "future"]);
}

#[tokio::test]
async fn compatible_edit_and_refresh_preserve_sparse_metadata_and_saved_identity() {
    let scratch = scratch_app();
    let full = json!({"data":[
        {"id":"guard/Combo","display_name":"Saved alias","capabilities":{"contextWindow":200000,"maxOutput":32000,"vision":true,"reasoning":true,"tools":true,"thinkingFormat":"openai","thinkingCanDisable":true}},
        {"id":"manual"}
    ]}).to_string();
    let sparse = json!({"data":[{"id":"guard/Combo","capabilities":{"vision":false,"reasoning":false,"tools":false}},{"id":"new"}]}).to_string();
    let (root, server) = serve(vec![("200 OK", full), ("200 OK", sparse.clone()), ("200 OK", sparse), ("503 Service Unavailable", "{}".into())]);
    let mut first = input("Historical label", "chat-completions", &format!("{root}/v1"), &["manual","guard/Combo"]);
    first.api_key = "fixture-key".into();
    first.integration = Some("durindoor".into());
    let kind = connect_custom(&scratch.0, first).await.unwrap();
    let before = scratch.0.credentials.lock().expect("credentials lock").custom[&kind].clone();
    let bot: crate::model::Bot = serde_json::from_value(json!({
        "id":"fixture-bot","name":"Fixture bot","description":"","symbol_name":"bolt","accent":"blue",
        "runner_id":"fixture-runner","provider":kind,"model":"guard/Combo","created_at":1.0
    })).unwrap();
    scratch.0.state.lock().expect("state lock").bots.push(bot.clone());
    let mut edit = input("Historical label", "chat-completions", &before.base_url, &["manual","guard/Combo"]);
    edit.kind = Some(kind.clone());
    edit.api_key = before.api_key.clone();
    connect_custom(&scratch.0, edit).await.unwrap();
    assert_eq!(refresh_custom_models(&scratch.0).await.unwrap(), 1);
    let saved = scratch.0.credentials.lock().expect("credentials lock").custom[&kind].clone();
    assert_eq!((saved.name.as_str(), saved.api, saved.api_key.as_str(), saved.created_at, saved.integration), (before.name.as_str(), before.api, before.api_key.as_str(), before.created_at, before.integration));
    assert_eq!(saved.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["manual","guard/Combo","new"]);
    assert_eq!((saved.models[1].name.as_deref(), saved.models[1].context_window, saved.models[1].max_output), (Some("Saved alias"),Some(200000),Some(32000)));
    assert_eq!((saved.models[1].images,saved.models[1].reasoning,saved.models[1].tools), (Some(false),Some(false),Some(false)));
    assert!(saved.levels(&saved.models[1]).is_empty());
    assert!(refresh_custom_models(&scratch.0).await.is_err());
    assert_eq!(scratch.0.credentials.lock().expect("credentials lock").custom[&kind], saved);
    assert_eq!(Credentials::load(&scratch.0.config).custom[&kind], saved);
    assert_eq!(scratch.0.state.lock().expect("state lock").bots[0], bot);
    assert!(server.join().unwrap().iter().all(|request| request.starts_with("GET /v1/models ")));
}

#[tokio::test]
async fn compatible_save_failure_keeps_in_memory_and_synced_credentials_unchanged() {
    let scratch = scratch_app();
    let before = serde_json::to_value(&*scratch.0.credentials.lock().expect("credentials lock")).unwrap();
    std::fs::create_dir(scratch.0.config.credentials_path()).unwrap();
    let (root, server) = serve(vec![("405 Method Not Allowed", "{}".into())]);
    assert_eq!(connect_custom(&scratch.0, input("OpenAI Compatible", "chat-completions", &root, &["manual"])).await.unwrap_err(), "Could not save provider");
    server.join().unwrap();
    assert_eq!(serde_json::to_value(&*scratch.0.credentials.lock().expect("credentials lock")).unwrap(), before);
}

#[tokio::test]
async fn compatible_connections_are_not_singletons_by_display_name() {
    let scratch = scratch_app();
    let (root, server) = serve(vec![("404 Not Found","{}".into()),("404 Not Found","{}".into())]);
    let a = connect_custom(&scratch.0, input("OpenAI Compatible","chat-completions",&root,&["guard"])).await.unwrap();
    let b = connect_custom(&scratch.0, input("OpenAI Compatible","responses",&root,&["guard"])).await.unwrap();
    server.join().unwrap();
    assert_ne!(a,b);
    let credentials = scratch.0.credentials.lock().expect("credentials lock");
    assert_eq!(credentials.custom[&a].name,credentials.custom[&b].name);
    assert_eq!(credentials.custom[&a].integration,None);
    assert_eq!(credentials.custom[&b].integration,None);
}

#[tokio::test]
async fn compatible_messages_discovery_rejects_missing_repeated_and_excessive_cursors() {
    let scratch = scratch_app();
    for pages in [
        vec![json!({"data":[{"id":"a"}],"has_more":true})],
        vec![json!({"data":[{"id":"a"}],"has_more":true,"last_id":"a"}); 2],
        (0..16).map(|i| json!({"data":[{"id":i.to_string()}],"has_more":true,"last_id":i.to_string()})).collect(),
    ] {
        let (root, server) = serve(pages.into_iter().map(|body| ("200 OK",body.to_string())).collect());
        let error = list_custom_models(&scratch.0,"Compatible","messages",&root,"",None).await.unwrap_err();
        assert!(error.contains("cursor") || error.contains("pagination limit"), "{error}");
        server.join().unwrap();
    }
}

#[tokio::test]
async fn compatible_failed_edit_persistence_retains_key_selection_metadata_and_stamp() {
    let scratch = scratch_app();
    let full = json!({"data":[{"id":"saved","capabilities":{"vision":true,"contextWindow":200000}}]}).to_string();
    let (root, server) = serve(vec![("200 OK",full),("405 Method Not Allowed","{}".into())]);
    let mut first = input("Saved name","responses",&root,&["saved"]);
    first.api_key = "fixture-original-key".into();
    let kind = connect_custom(&scratch.0,first).await.unwrap();
    let before = serde_json::to_value(&*scratch.0.credentials.lock().expect("credentials lock")).unwrap();
    std::fs::remove_file(scratch.0.config.credentials_path()).unwrap();
    std::fs::create_dir(scratch.0.config.credentials_path()).unwrap();
    let mut edit = input("Changed name","responses",&root,&["different"]);
    edit.kind = Some(kind);
    edit.api_key = "fixture-replacement-key".into();
    assert_eq!(connect_custom(&scratch.0,edit).await.unwrap_err(),"Could not save provider");
    server.join().unwrap();
    assert_eq!(serde_json::to_value(&*scratch.0.credentials.lock().expect("credentials lock")).unwrap(),before);
}

#[tokio::test]
async fn compatible_discovery_bounds_total_bytes_across_pages() {
    let scratch = scratch_app();
    let page = json!({"data":[{"id":"a"}],"has_more":true,"last_id":"a"}).to_string();
    let first = format!("{page}{}", " ".repeat(MODEL_RESPONSE_LIMIT / 2));
    let second = format!("{{\"data\":[{{\"id\":\"b\"}}],\"has_more\":false}}{}", " ".repeat(MODEL_RESPONSE_LIMIT / 2));
    let (root,server) = serve(vec![("200 OK",first),("200 OK",second)]);
    assert!(list_custom_models(&scratch.0,"Compatible","messages",&root,"",None).await.unwrap_err().contains("too large"));
    server.join().unwrap();
}
