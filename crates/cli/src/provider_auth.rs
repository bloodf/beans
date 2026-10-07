//! Connects and disconnects the account's provider credentials on any Device. API keys are
//! checked here before they enter the encrypted `credentials` blob, and a custom provider's
//! server answers for its key and lists its models. Subscription sign-ins use the providers'
//! PKCE loopback flows; a desktop opens the URL itself and a phone hands it to its native
//! in-app browser.

use std::sync::Arc;

use serde_json::Value;

use lorca_provider_auth::chatgpt::{self as chatgpt_oauth, ChatGptTokens};
use lorca_provider_auth::grok::{self as grok_oauth, GrokTokens};

use crate::app::App;
use crate::config;
use crate::credentials::{is_custom, ApiKeyCredential, Credentials, CustomApi, CustomIntegration, CustomModel, CustomProvider, CUSTOM_PREFIX, PROVIDER_KINDS};

const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const OPENCODE_BASE_URL: &str = "https://opencode.ai/zen";
const OPENCODE_GO_BASE_URL: &str = "https://opencode.ai/zen/go";
const MODEL_RESPONSE_LIMIT: usize = 1024 * 1024;

fn discovery_client() -> Result<reqwest::Client, String> {
    lorca_tls::client_builder().redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(20)).build()
        .map_err(|_| "Could not initialize provider connection".into())
}

async fn bounded_json(mut response: reqwest::Response, name: &str, limit: usize) -> Result<(Value, usize), String> {
    if response.content_length().is_some_and(|length| length > limit as u64) {
        return Err(format!("{name} response is too large"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| format!("Could not read {name} response"))? {
        if bytes.len().saturating_add(chunk.len()) > limit { return Err(format!("{name} response is too large")); }
        bytes.extend_from_slice(&chunk);
    }
    let value = serde_json::from_slice(&bytes).map_err(|_| format!("{name} did not answer like an API. Check the base URL."))?;
    Ok((value, bytes.len()))
}

fn integration_root(api: CustomApi, base_url: &str, integration: Option<CustomIntegration>) -> Result<String, String> {
    if integration != Some(CustomIntegration::Durindoor) { return custom_root(api, base_url); }
    if api != CustomApi::ChatCompletions { return Err("DurinDoor uses OpenAI Chat Completions".into()); }
    let root = custom_root(api, base_url)?;
    let root = ["/models", "/realtime/auth", "/responses", "/messages"].iter()
        .find_map(|suffix| root.strip_suffix(suffix)).unwrap_or(&root);
    Ok(if root.ends_with("/v1") { root.to_string() } else { format!("{root}/v1") })
}


fn env_url(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.trim().is_empty()).map(|s| s.trim_end_matches('/').to_string())
}

fn deepseek_base_url() -> String {
    env_url("LORCA_DEEPSEEK_BASE_URL").unwrap_or_else(|| "https://api.deepseek.com".into())
}

fn anthropic_base_url() -> String {
    env_url("LORCA_ANTHROPIC_BASE_URL").unwrap_or_else(|| ANTHROPIC_BASE_URL.to_string())
}

/// Removes an optional `/v1` from a custom OpenCode root; the credential check adds the
/// endpoint it needs.
fn opencode_root(root: &str) -> &str {
    root.trim_end_matches('/').strip_suffix("/v1").unwrap_or_else(|| root.trim_end_matches('/'))
}

/// A base URL the user typed: blank means the default; otherwise an http(s) root without a
/// trailing slash.
fn custom_base_url(base_url: Option<&str>) -> Result<Option<String>, String> {
    let Some(url) = base_url.map(str::trim).filter(|u| !u.is_empty()) else { return Ok(None) };
    let parsed = reqwest::Url::parse(url).map_err(|_| "The base URL must be a valid http:// or https:// URL")?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("The base URL must start with http:// or https://".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("Use a base URL without credentials, a query, or a fragment. Put the key in the API key field.".into());
    }
    Ok(Some(parsed.as_str().trim_end_matches('/').to_string()))
}

/// Checks a DeepSeek key against the API (the given root, or DeepSeek's) before saving both.
pub async fn connect_deepseek(app: &Arc<App>, api_key: &str, base_url: Option<&str>) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Paste a DeepSeek API key".into());
    }
    let base_url = custom_base_url(base_url)?;
    let root = base_url.clone().unwrap_or_else(deepseek_base_url);
    let request = app.http.get(format!("{}/models", root.trim_end_matches("/anthropic"))).bearer_auth(key);
    check_key("DeepSeek", request).await?;
    save_api_key(app, "deepseek", key, base_url)
}

/// Checks an Anthropic key against the API (the given root, or Anthropic's) before saving both.
pub async fn connect_anthropic(app: &Arc<App>, api_key: &str, base_url: Option<&str>) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Paste an Anthropic API key".into());
    }
    let base_url = custom_base_url(base_url)?;
    let root = base_url.clone().unwrap_or_else(anthropic_base_url);
    let request = app.http.get(format!("{root}/v1/models")).header("x-api-key", key).header("anthropic-version", ANTHROPIC_VERSION);
    check_key("Anthropic", request).await?;
    save_api_key(app, "anthropic", key, base_url)
}

/// Checks an OpenCode Zen key. The model catalog is public, so the account check uses Go's
/// authenticated usage endpoint; a valid Zen key without a Go subscription answers 403 after
/// authentication and is still a valid Zen credential.
pub async fn connect_opencode(app: &Arc<App>, api_key: &str, base_url: Option<&str>) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Paste an OpenCode Zen API key".into());
    }
    let base_url = custom_base_url(base_url)?;
    let configured = base_url.clone().or_else(|| env_url("LORCA_OPENCODE_BASE_URL"));
    let root = configured.as_deref().map(opencode_root).unwrap_or(OPENCODE_BASE_URL);
    if root == OPENCODE_BASE_URL {
        let request = app.http.get(format!("{root}/go/v1/usage")).bearer_auth(key);
        check_opencode_key("OpenCode Zen", request, false).await?;
    } else {
        check_key("OpenCode Zen", app.http.get(format!("{root}/v1/models")).bearer_auth(key)).await?;
    }
    save_api_key(app, "opencode", key, base_url)
}

/// Checks both the OpenCode key and its Go entitlement against the authenticated usage route.
pub async fn connect_opencode_go(app: &Arc<App>, api_key: &str, base_url: Option<&str>) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Paste an OpenCode Go API key".into());
    }
    let base_url = custom_base_url(base_url)?;
    let root = base_url
        .clone()
        .or_else(|| env_url("LORCA_OPENCODE_GO_BASE_URL"))
        .unwrap_or_else(|| OPENCODE_GO_BASE_URL.into());
    let root = opencode_root(&root);
    if root == OPENCODE_GO_BASE_URL {
        check_opencode_key("OpenCode Go", app.http.get(format!("{root}/v1/usage")).bearer_auth(key), true).await?;
    } else {
        check_key("OpenCode Go", app.http.get(format!("{root}/v1/models")).bearer_auth(key)).await?;
    }
    save_api_key(app, "opencode-go", key, base_url)
}

async fn check_key(name: &str, request: reqwest::RequestBuilder) -> Result<(), String> {
    let response = request.send().await.map_err(|e| format!("{name} unreachable: {}", lorca_tls::describe(&e)))?;
    match response.status() {
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => Err(format!("{name} rejected that key")),
        status if status.is_success() => Ok(()),
        status => Err(format!("{name} answered {status}")),
    }
}

async fn check_opencode_key(name: &str, request: reqwest::RequestBuilder, requires_go: bool) -> Result<(), String> {
    let response = request.send().await.map_err(|e| format!("{name} unreachable: {}", lorca_tls::describe(&e)))?;
    match response.status() {
        reqwest::StatusCode::UNAUTHORIZED => Err(format!("{name} rejected that key")),
        reqwest::StatusCode::FORBIDDEN if requires_go => Err("OpenCode Go needs an active subscription".into()),
        reqwest::StatusCode::FORBIDDEN => Ok(()),
        status if status.is_success() => Ok(()),
        status => Err(format!("{name} answered {status}")),
    }
}

fn save_api_key(app: &Arc<App>, kind: &str, key: &str, base_url: Option<String>) -> Result<(), String> {
    let credential = Some(ApiKeyCredential { api_key: key.to_string(), base_url, connected_at: config::now_unix() });
    let update = |credentials: &mut Credentials| match kind {
        "deepseek" => credentials.deepseek = credential,
        "anthropic" => credentials.anthropic = credential,
        "opencode" => credentials.opencode = credential,
        "opencode-go" => credentials.opencode_go = credential,
        _ => unreachable!(),
    };
    app.update_credentials(kind, update).map_err(|e| e.to_string())
}

/// What the user typed for a custom provider.
#[derive(Debug, Default)]
pub struct CustomInput {
    /// The provider being edited; `None` adds one.
    pub kind: Option<String>,
    pub integration: Option<String>,
    pub name: String,
    pub api: String,
    pub base_url: String,
    pub api_key: String,
    /// Model ids in the user's order. Empty takes every model the server lists.
    pub models: Vec<String>,
}

/// Checks a custom provider's server and saves the provider for the account, answering with
/// its kind. The model list the server publishes checks the key and tells each model's window,
/// output cap, and whether it sees images; a server without one still works with the model ids
/// the user gave.
pub async fn connect_custom(app: &Arc<App>, input: CustomInput) -> Result<String, String> {
    let snapshot = input.kind.as_ref().and_then(|kind| app.credentials.lock().expect("credentials lock").custom.get(kind).cloned());
    let requested_integration = CustomIntegration::parse(input.integration.as_deref())?;
    let integration = requested_integration.or_else(|| snapshot.as_ref().and_then(|provider| provider.integration));
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("Name the provider".into());
    }
    if name.chars().count() > 40 {
        return Err("Keep the name under 40 characters".into());
    }
    let api = CustomApi::parse(input.api.trim()).ok_or_else(|| format!("Unknown API {}", input.api.trim()))?;
    let base_url = integration_root(api, &input.base_url, integration)?;
    let api_key = input.api_key.trim().to_string();
    let mut ids: Vec<String> = Vec::new();
    for id in input.models.iter().filter(|id| !id.trim().is_empty()) {
        if !ids.iter().any(|seen| seen == id) {
            ids.push(id.to_string());
        }
    }
    if let Some(kind) = &input.kind {
        if !is_custom(kind) {
            return Err(format!("{kind} is not a custom provider"));
        }
    }
    {
        let credentials = app.credentials.lock().unwrap();
        let taken = PROVIDER_KINDS.iter().any(|kind| credentials.label(kind).eq_ignore_ascii_case(&name));
        if taken {
            return Err(format!("A provider named {name} exists already"));
        }
    }

    let listed = list_models(&name, api, &base_url, &api_key).await?;
    let models = if ids.is_empty() {
        let listed = listed.ok_or_else(|| format!("{name} publishes no model list. Add the model ids yourself."))?;
        let chat: Vec<CustomModel> = listed.into_iter().filter(|model| !model.0).map(|(_, discovered)| {
            match snapshot.as_ref().and_then(|provider| provider.models.iter().find(|model| model.id == discovered.id)) {
                Some(saved) => {
                    let mut model = saved.clone();
                    model.merge_metadata(&discovered);
                    model
                }
                None => discovered,
            }
        }).collect();
        if chat.is_empty() {
            return Err(format!("{name} lists no models. Add the model ids yourself."));
        }
        chat
    } else {
        let listed = listed.unwrap_or_default();
        ids.into_iter().map(|id| {
            let mut model = snapshot.as_ref().and_then(|provider| provider.models.iter().find(|model| model.id == id)).cloned()
                .unwrap_or_else(|| CustomModel { id, ..Default::default() });
            if let Some((_, discovered)) = listed.iter().find(|(_, discovered)| discovered.id == model.id) {
                model.merge_metadata(discovered);
            }
            model
        }).collect()
    };

    let kind = {
        let mut credentials = app.credentials.lock().expect("credentials lock");
        let kind = input.kind.clone().unwrap_or_else(|| custom_kind(&credentials, &name));
        let created_at = credentials.custom.get(&kind).map(|provider| provider.created_at).unwrap_or_else(config::now_unix);
        if input.kind.is_some() && credentials.custom.get(&kind) != snapshot.as_ref() {
            return Err("The provider changed while connecting. Open it again.".into());
        }
        let provider = CustomProvider { name, api, base_url, api_key, models, created_at, integration };
        let mut next = credentials.clone();
        next.custom.insert(kind.clone(), provider);
        next.touch(&kind);
        next.save(&app.config).map_err(|_| "Could not save provider")?;
        *credentials = next;
        kind
    };
    app.push_credentials();
    app.emit(app.roster_summary());
    Ok(kind)
}

/// The chat models a custom provider's server lists, for the apps' model picker: `None` when
/// the server publishes no list. The base URL is read as `connect_custom` reads it, and a key
/// the server refuses or a server that cannot be reached fails the same way.
pub async fn list_custom_models(_app: &Arc<App>, name: &str, api: &str, base_url: &str, api_key: &str, integration: Option<&str>) -> Result<Option<Vec<CustomModel>>, String> {
    let name = Some(name.trim()).filter(|name| !name.is_empty()).unwrap_or("The server");
    let api = CustomApi::parse(api.trim()).ok_or_else(|| format!("Unknown API {}", api.trim()))?;
    let integration = CustomIntegration::parse(integration)?;
    let root = integration_root(api, base_url, integration)?;
    let listed = list_models(name, api, &root, api_key.trim()).await?;
    Ok(listed.map(|models| models.into_iter().filter(|(not_chat, _)| !not_chat).map(|(_, model)| model).collect()))
}

/// Refreshes saved custom-provider catalogs. Each fetch uses a snapshot, then commits only if
/// that provider has not changed meanwhile; no response is allowed to restore a deleted or
/// edited provider. Only actual model changes are stamped, saved and synced.
pub async fn refresh_custom_models(app: &Arc<App>) -> Result<usize, String> {
    let providers: Vec<(String, CustomProvider, Option<f64>)> = {
        let credentials = app.credentials.lock().unwrap();
        credentials.custom.iter().map(|(kind, provider)| (kind.clone(), provider.clone(), credentials.changed_at.get(kind).copied())).collect()
    };
    let mut changed = 0;
    let mut failed = None;
    for (kind, snapshot, stamp) in providers {
        let discovery = match integration_root(snapshot.api, &snapshot.base_url, snapshot.integration) {
            Ok(root) => list_models(&snapshot.name, snapshot.api, &root, &snapshot.api_key).await,
            Err(error) => Err(error),
        };
        let listed = match discovery {
            Ok(Some(listed)) => listed,
            Ok(None) => continue,
            Err(_) => {
                failed.get_or_insert_with(|| format!("Could not refresh models for {kind}"));
                continue;
            }
        };
        let models = listed.into_iter().filter(|(not_chat, _)| !not_chat).map(|(_, model)| model).collect();
        let mut credentials = app.credentials.lock().unwrap();
        if credentials.changed_at.get(&kind).copied() != stamp {
            continue;
        }
        let Some(current) = credentials.custom.get_mut(&kind) else { continue };
        if *current != snapshot {
            continue;
        }
        let previous = current.clone();
        if current.merge_discovered_models(models) {
            let previous_stamp = credentials.changed_at.get(&kind).copied();
            credentials.touch(&kind);
            if credentials.save(&app.config).is_err() {
                credentials.custom.insert(kind.clone(), previous);
                match previous_stamp {
                    Some(stamp) => { credentials.changed_at.insert(kind, stamp); }
                    None => { credentials.changed_at.remove(&kind); }
                }
                return Err("Could not save refreshed models".into());
            }
            drop(credentials);
            app.push_credentials();
            app.emit(app.roster_summary());
            changed += 1;
        }
    }
    if let Some(error) = failed { Err(error) } else { Ok(changed) }
}

/// A new custom provider's kind: `custom:` and a slug of its name, with a number when another
/// provider has it. A provider deleted under that slug gives it up, so its bots run again.
fn custom_kind(credentials: &Credentials, name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(32).collect();
    let base = format!("{CUSTOM_PREFIX}{}", if slug.is_empty() { "provider" } else { slug.trim_end_matches('-') });
    let mut kind = base.clone();
    let mut n = 2;
    while credentials.custom.contains_key(&kind) {
        kind = format!("{base}-{n}");
        n += 1;
    }
    kind
}

/// A custom provider's base URL: required, http(s), and cut back to the root when the user
/// pasted a whole endpoint, since each adapter adds the path its protocol needs.
fn custom_root(api: CustomApi, base_url: &str) -> Result<String, String> {
    let url = custom_base_url(Some(base_url))?.ok_or("Enter the server's base URL")?;
    let endpoint: &[&str] = match api {
        CustomApi::ChatCompletions => &["/chat/completions", "/models"],
        CustomApi::Responses => &["/responses", "/models"],
        CustomApi::Messages => &["/v1/messages", "/v1/models", "/v1", "/messages", "/models"],
    };
    Ok(endpoint.iter().find_map(|path| url.strip_suffix(path)).unwrap_or(&url).to_string())
}

/// The models a custom provider's server lists, each with whether it is not for chat
/// (embeddings, speech, images); `None` when the server publishes no list. A key the server
/// refuses, or a server that cannot be reached, fails.
async fn list_models(name: &str, api: CustomApi, root: &str, api_key: &str) -> Result<Option<Vec<(bool, CustomModel)>>, String> {
    const MAX_PAGES: usize = 16;
    let client = discovery_client()?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut cursor = None;
    let mut cursors = std::collections::HashSet::new();
    let mut seen = std::collections::HashSet::new();
    let mut models = Vec::new();
    let mut remaining_bytes = MODEL_RESPONSE_LIMIT;
    for page in 0..MAX_PAGES {
        let mut request = match api {
            CustomApi::ChatCompletions | CustomApi::Responses => {
                let request = client.get(format!("{root}/models"));
                if api_key.is_empty() { request } else { request.bearer_auth(api_key) }
            }
            CustomApi::Messages => {
                let mut request = client.get(format!("{root}/v1/models")).query(&[("limit", "1000")])
                    .header("anthropic-version", ANTHROPIC_VERSION);
                if let Some(cursor) = &cursor { request = request.query(&[("after_id", cursor)]); }
                if api_key.is_empty() { request } else { request.header("x-api-key", api_key) }
            }
        };
        request = request.timeout(deadline.saturating_duration_since(tokio::time::Instant::now()));
        let response = request.send().await.map_err(|_| format!("{name} unreachable. Check the URL on this Device."))?;
        match response.status() {
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED if page == 0 => return Ok(None),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => return Err(if api_key.is_empty() { format!("{name} needs an API key") } else { format!("{name} rejected that key") }),
            status if !status.is_success() => return Err(format!("{name} model discovery failed ({status})")),
            _ => {}
        }
        let (body, bytes) = bounded_json(response, name, remaining_bytes).await?;
        remaining_bytes -= bytes;
        let listed = listed_models(&body).ok_or_else(|| format!("{name} did not answer with a model list"))?;
        models.extend(listed.into_iter().filter(|(_, model)| seen.insert(model.id.clone())));
        if body.get("has_more").is_some_and(|value| !value.is_boolean()) {
            return Err(format!("{name} model list has invalid pagination metadata"));
        }
        if api != CustomApi::Messages || body.get("has_more").and_then(Value::as_bool) != Some(true) {
            return Ok(Some(models));
        }
        let next = body["last_id"].as_str().filter(|id| !id.is_empty())
            .ok_or_else(|| format!("{name} model list is missing its pagination cursor"))?;
        if !cursors.insert(next.to_string()) { return Err(format!("{name} model list repeated its pagination cursor")); }
        cursor = Some(next.to_string());
    }
    Err(format!("{name} model list exceeds the pagination limit"))
}

/// A model list in the shape OpenAI, Anthropic, and most gateways and local servers answer
/// with: `data`, `models`, or a bare array of entries.
fn listed_models(body: &Value) -> Option<Vec<(bool, CustomModel)>> {
    let entries = body.get("data").or_else(|| body.get("models")).unwrap_or(body).as_array()?;
    let mut seen = std::collections::HashSet::new();
    let models = entries.iter().map(listed_model).collect::<Option<Vec<_>>>()?;
    Some(models.into_iter().filter(|(_, model)| seen.insert(model.id.clone())).collect())
}


/// The `type` a model list (Together's) gives a model chat cannot use.
const NOT_CHAT_TYPES: [&str; 8] = ["embedding", "rerank", "image", "audio", "transcribe", "moderation", "video", "tts"];

/// One entry of a model list, read for the fields servers use for a model's name, window,
/// output cap, and inputs, and whether it is a model chat cannot use.
fn listed_model(entry: &Value) -> Option<(bool, CustomModel)> {
    let id = entry["id"].as_str().or_else(|| entry["name"].as_str()).filter(|id| !id.trim().is_empty())?.to_string();
    let number = |paths: &[&str]| paths.iter().find_map(|path| entry.pointer(path).and_then(Value::as_u64)).filter(|n| *n > 0);
    let name = ["display_name", "name"]
        .iter()
        .find_map(|key| entry[*key].as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty() && *name != id)
        .map(str::to_string);
    let context_window = number(&["/context_length", "/context_window", "/max_model_len", "/max_context_length", "/max_input_tokens", "/top_provider/context_length", "/capabilities/contextWindow", "/contextLength"]);
    let max_output = number(&["/max_output_tokens", "/max_completion_tokens", "/top_provider/max_completion_tokens", "/capabilities/maxOutput"]);
    let inputs = entry.pointer("/architecture/input_modalities").or_else(|| entry.pointer("/modalities/input")).and_then(Value::as_array);
    let images = entry.pointer("/capabilities/vision").and_then(Value::as_bool).or_else(|| inputs.map(|inputs| inputs.iter().any(|input| input == "image")));
    let not_chat = entry.pointer("/capabilities/completion_chat").and_then(Value::as_bool) == Some(false)
        || entry["kind"].as_str().is_some_and(|kind| NOT_CHAT_TYPES.contains(&kind) || matches!(kind, "webSearch" | "imageGeneration" | "textToSpeech" | "speechToText" | "completion"))
        || entry["type"].as_str().is_some_and(|kind| NOT_CHAT_TYPES.contains(&kind))
        || entry.pointer("/architecture/output_modalities").and_then(Value::as_array).is_some_and(|outputs| !outputs.iter().any(|output| output == "text"));
    let reasoning = entry.pointer("/capabilities/reasoning").and_then(Value::as_bool);
    let tools = entry.pointer("/capabilities/tools").and_then(Value::as_bool);
    let thinking_format = entry.pointer("/capabilities/thinkingFormat").and_then(Value::as_str).map(str::to_string);
    let thinking_can_disable = entry.pointer("/capabilities/thinkingCanDisable").and_then(Value::as_bool);
    Some((not_chat, CustomModel { id, name, context_window, max_output, images, reasoning, tools, thinking_format, thinking_can_disable }))
}

/// Runs a ChatGPT sign-in, opening its authorization URL through the Device's UI.
pub async fn connect_chatgpt(
    app: &Arc<App>,
    open_url: impl FnOnce(&str) -> Result<(), String>,
) -> Result<ChatGptTokens, String> {
    let tokens = chatgpt_oauth::login(&app.http, open_url, std::time::Duration::from_secs(5 * 60)).await?;
    app.update_credentials("chatgpt", |c| c.chatgpt = Some(tokens.clone())).map_err(|e| e.to_string())?;
    Ok(tokens)
}

/// Runs a Grok sign-in, opening its authorization URL through the Device's UI.
pub async fn connect_grok(
    app: &Arc<App>,
    open_url: impl FnOnce(&str) -> Result<(), String>,
) -> Result<GrokTokens, String> {
    let endpoints = env_url("LORCA_GROK_ISSUER").map(|issuer| grok_oauth::Endpoints::at(&issuer)).unwrap_or_else(grok_oauth::Endpoints::xai);
    let tokens = grok_oauth::login(&app.http, &endpoints, open_url, std::time::Duration::from_secs(5 * 60)).await?;
    app.update_credentials("grok", |c| c.grok = Some(tokens.clone())).map_err(|e| e.to_string())?;
    Ok(tokens)
}

/// Disconnects `kind` for the whole account: every Device drops the credential, and a custom
/// provider is deleted.
pub fn disconnect(app: &Arc<App>, kind: &str) -> Result<(), String> {
    if is_custom(kind) {
        if !app.credentials.lock().unwrap().custom.contains_key(kind) {
            return Err(format!("Unknown provider {kind}"));
        }
        return app
            .update_credentials(kind, |credentials| {
                credentials.custom.remove(kind);
            })
            .map_err(|e| e.to_string());
    }
    if !PROVIDER_KINDS.contains(&kind) {
        return Err(format!("Unknown provider {kind}"));
    }
    app.update_credentials(kind, |credentials| match kind {
        "deepseek" => credentials.deepseek = None,
        "anthropic" => credentials.anthropic = None,
        "opencode" => credentials.opencode = None,
        "opencode-go" => credentials.opencode_go = None,
        "chatgpt" => credentials.chatgpt = None,
        "grok" => {
            // Tell xAI the sign-in is over; the account-wide removal stands either way.
            if let Some(tokens) = credentials.grok.take() {
                let http = app.http.clone();
                let endpoints = env_url("LORCA_GROK_ISSUER").map(|issuer| grok_oauth::Endpoints::at(&issuer)).unwrap_or_else(grok_oauth::Endpoints::xai);
                tokio::spawn(async move {
                    if let Err(error) = grok_oauth::revoke(&http, &endpoints, &tokens.refresh_token).await {
                        tracing::debug!("grok revoke: {error}");
                    }
                });
            }
        }
        _ => unreachable!(),
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
