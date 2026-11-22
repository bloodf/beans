//! The built-in providers' key checks (issue #601): the pasted key, the address it is checked
//! at, how the answer reads, and that a check never follows a redirect. Servers are loopback
//! fixtures reached through the `base_url` parameter; nothing reaches a real provider.

use super::tests::{join_server, scratch_app, serve};
use super::*;

/// A listener nobody should connect to, and whether anybody did.
fn silent() -> (String, std::net::TcpListener) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    (format!("http://{}", listener.local_addr().unwrap()), listener)
}

fn connected(listener: &std::net::TcpListener) -> bool {
    listener.accept().is_ok()
}

#[test]
fn anthropic_roots_drop_the_version_path() {
    assert_eq!(anthropic_root("https://proxy.example/v1/"), "https://proxy.example");
    assert_eq!(anthropic_root("https://proxy.example/v1"), "https://proxy.example");
    assert_eq!(anthropic_root("https://proxy.example/anthropic"), "https://proxy.example/anthropic");
    assert_eq!(anthropic_root("https://api.anthropic.com"), "https://api.anthropic.com");
}

#[test]
fn statuses_read_as_the_user_needs() {
    use reqwest::StatusCode as S;
    assert_eq!(key_check_status("DeepSeek", S::OK, None), Ok(()));
    assert_eq!(key_check_status("DeepSeek", S::UNAUTHORIZED, None).unwrap_err(), "DeepSeek rejected that key");
    assert_eq!(key_check_status("DeepSeek", S::FORBIDDEN, None).unwrap_err(), "DeepSeek rejected that key");
    assert_eq!(key_check_status("DeepSeek", S::INTERNAL_SERVER_ERROR, None).unwrap_err(), "DeepSeek answered 500 Internal Server Error");
    assert!(key_check_status("Anthropic", S::TEMPORARY_REDIRECT, None).unwrap_err().contains("redirected"));
    assert_eq!(key_check_status("OpenCode Zen", S::FORBIDDEN, Some(false)), Ok(()), "a Zen key without Go");
    assert_eq!(key_check_status("OpenCode Go", S::FORBIDDEN, Some(true)).unwrap_err(), "OpenCode Go needs an active subscription");
    assert_eq!(key_check_status("OpenCode Go", S::UNAUTHORIZED, Some(true)).unwrap_err(), "OpenCode Go rejected that key");
}

#[test]
fn opencode_keys_are_checked_where_a_key_matters() {
    let usage = format!("{OPENCODE_GO_BASE_URL}/v1/usage");
    // OpenCode's own service, however the address was pasted: the authenticated usage route.
    for root in [OPENCODE_BASE_URL, "https://opencode.ai/zen/", "https://opencode.ai/zen/v1", OPENCODE_GO_BASE_URL, "https://opencode.ai/zen/go/v1"] {
        assert_eq!(opencode_check(root, true), (usage.clone(), Some(true)), "{root}");
        assert_eq!(opencode_check(root, false), (usage.clone(), Some(false)), "{root}");
    }
    // Another server (a proxy) lists models.
    assert_eq!(opencode_check("http://127.0.0.1:9/v1", true), ("http://127.0.0.1:9/v1/models".into(), None));
}

#[tokio::test]
async fn a_key_with_invalid_characters_is_refused_before_any_request() {
    let scratch = scratch_app();
    let (root, listener) = silent();
    for key in ["sk-a\nb", "sk-a b", "sk-ä", "sk-\u{200B}x"] {
        for result in [
            connect_deepseek(&scratch.0, key, Some(&root)).await,
            connect_anthropic(&scratch.0, key, Some(&root)).await,
            connect_opencode(&scratch.0, key, Some(&root)).await,
            connect_opencode_go(&scratch.0, key, Some(&root)).await,
        ] {
            assert_eq!(result.unwrap_err(), "The key contains invalid characters. Copy it again from the provider.", "{key:?}");
        }
        let listed = list_custom_models(&scratch.0, "Gateway", "chat-completions", &root, key, None).await;
        assert!(listed.unwrap_err().contains("invalid characters"));
    }
    assert_eq!(connect_deepseek(&scratch.0, "  ", Some(&root)).await.unwrap_err(), "Paste a DeepSeek API key");
    assert!(!connected(&listener), "no request left this Runner");
    assert!(scratch.0.credentials.lock().unwrap().deepseek.is_none());
}

#[tokio::test]
async fn anthropic_accepts_a_pasted_v1_and_saves_the_root() {
    let scratch = scratch_app();
    let (root, server) = serve(vec![("200 OK", "{\"data\":[]}".into())]);
    connect_anthropic(&scratch.0, " sk-ant-fixture ", Some(&format!("{root}/v1/"))).await.unwrap();
    let requests = join_server(server).await;
    assert!(requests[0].starts_with("GET /v1/models "), "{}", requests[0]);
    assert!(requests[0].to_ascii_lowercase().contains("x-api-key: sk-ant-fixture"));
    let credentials = scratch.0.credentials.lock().unwrap();
    let saved = credentials.anthropic.as_ref().unwrap();
    assert_eq!((saved.api_key.as_str(), saved.base_url.as_deref()), ("sk-ant-fixture", Some(root.as_str())));
}

#[tokio::test]
async fn a_key_check_never_follows_a_redirect() {
    let scratch = scratch_app();
    let (target, listener) = silent();
    // The fixture writes the status line as given, so it carries the Location header too.
    let redirect: &'static str = Box::leak(format!("307 Temporary Redirect\r\nLocation: {target}/v1/models").into_boxed_str());
    let (root, server) = serve(vec![(redirect, String::new())]);
    let error = connect_anthropic(&scratch.0, "sk-ant-fixture", Some(&root)).await.unwrap_err();
    assert!(error.contains("redirected"), "{error}");
    join_server(server).await;
    assert!(!connected(&listener), "the key never went to the redirect's target");
    assert!(scratch.0.credentials.lock().unwrap().anthropic.is_none());
}

#[tokio::test]
async fn rejections_and_outages_are_reported_and_nothing_is_saved() {
    let scratch = scratch_app();
    for (status, expected) in [("401 Unauthorized", "DeepSeek rejected that key"), ("403 Forbidden", "DeepSeek rejected that key"), ("500 Internal Server Error", "DeepSeek answered 500 Internal Server Error")] {
        let (root, server) = serve(vec![(status, "{}".into())]);
        assert_eq!(connect_deepseek(&scratch.0, "sk-fixture", Some(&root)).await.unwrap_err(), expected);
        join_server(server).await;
    }
    assert!(scratch.0.credentials.lock().unwrap().deepseek.is_none());
    let (root, server) = serve(vec![("200 OK", "{\"data\":[]}".into())]);
    connect_deepseek(&scratch.0, "sk-fixture", Some(&root)).await.unwrap();
    assert!(join_server(server).await[0].starts_with("GET /models "));
    assert_eq!(scratch.0.credentials.lock().unwrap().deepseek.as_ref().unwrap().api_key, "sk-fixture");
}

#[tokio::test]
async fn custom_model_ids_are_trimmed_and_deduplicated() {
    let scratch = scratch_app();
    let (root, server) = serve(vec![("404 Not Found", "{}".into())]);
    let input = CustomInput { name: "Local".into(), api: "chat-completions".into(), base_url: root, models: vec![" gpt-4o ".into(), "gpt-4o".into(), "  ".into(), "qwen3".into()], ..Default::default() };
    let kind = connect_custom(&scratch.0, input).await.unwrap();
    join_server(server).await;
    let credentials = scratch.0.credentials.lock().unwrap();
    let ids: Vec<&str> = credentials.custom[&kind].models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(ids, ["gpt-4o", "qwen3"]);
}
