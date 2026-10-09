//! Runtime providers built from the account's credentials. Credential setup lives in
//! `provider_auth`, which is also linked by Devices that never run a bot.

use std::sync::Arc;

use async_trait::async_trait;

use beans_agent::models::{ModelInfo, Rates, ThinkingMode, Wire};
use beans_agent::providers::anthropic::ANTHROPIC_BASE_URL;
use beans_agent::providers::{
    AnthropicProvider, ChatGptProvider, ChatGptTokens, GrokProvider, GrokTokenSource, GrokTokens, OpenAiCompatProvider,
    OpenAiResponsesProvider, TokenSource,
};
use beans_agent::{models, AssistantEventStream, ModelRequest, Provider, ThinkingLevel};
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::credentials::{is_custom, CustomApi, CustomProvider};

pub const OPENCODE_BASE_URL: &str = "https://opencode.ai/zen";
pub const OPENCODE_GO_BASE_URL: &str = "https://opencode.ai/zen/go";

const USER_AGENT: &str = concat!("beans/", env!("CARGO_PKG_VERSION"));

/// Identifies Beans to OpenCode and sends the session header it uses for routing and prompt
/// caching. The regular request options also send `x-session-affinity`.
struct OpenCodeHeaders {
    inner: Arc<dyn Provider>,
}

#[async_trait]
impl Provider for OpenCodeHeaders {
    fn provider_id(&self) -> &str {
        self.inner.provider_id()
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    fn supports_images(&self) -> bool {
        self.inner.supports_images()
    }

    fn model_info(&self) -> Option<&beans_agent::models::ModelInfo> {
        self.inner.model_info()
    }

    async fn stream(&self, mut request: ModelRequest, cancel: CancellationToken) -> AssistantEventStream {
        request.options.headers.insert("User-Agent".into(), USER_AGENT.into());
        if let Some(session_id) = request.options.session_id.clone() {
            request.options.headers.insert("x-opencode-session".into(), session_id);
        }
        self.inner.stream(request, cancel).await
    }
}

fn opencode_headers(inner: Arc<dyn Provider>) -> Arc<dyn Provider> {
    Arc::new(OpenCodeHeaders { inner })
}

/// Reads ChatGPT tokens from the account's credentials and hands refreshed ones to every Device.
pub struct AppTokenSource(pub Arc<App>);

#[async_trait]
impl TokenSource for AppTokenSource {
    async fn tokens(&self) -> Result<ChatGptTokens, String> {
        self.0.credentials.lock().unwrap().chatgpt.clone().ok_or_else(|| "ChatGPT is not connected".to_string())
    }

    async fn store(&self, tokens: ChatGptTokens) -> Result<(), String> {
        self.0.update_credentials("chatgpt", |c| c.chatgpt = Some(tokens)).map_err(|e| e.to_string())
    }
}

/// Reads Grok tokens from the account's credentials and hands refreshed ones to every Device.
pub struct AppGrokTokenSource(pub Arc<App>);

#[async_trait]
impl GrokTokenSource for AppGrokTokenSource {
    async fn tokens(&self) -> Result<GrokTokens, String> {
        self.0.credentials.lock().unwrap().grok.clone().ok_or_else(|| "Grok is not connected".to_string())
    }

    async fn store(&self, tokens: GrokTokens) -> Result<(), String> {
        self.0.update_credentials("grok", |c| c.grok = Some(tokens)).map_err(|e| e.to_string())
    }
}

/// Whether the model takes image content parts. A text-only model rejects the whole request,
/// so a Runner sends it the attachment's path alone.
pub fn supports_vision(app: &App, kind: &str, model: Option<&str>) -> bool {
    if is_custom(kind) {
        let credentials = app.credentials.lock().unwrap();
        let Some(provider) = credentials.custom.get(kind) else { return false };
        let model = model.filter(|m| !m.trim().is_empty()).or_else(|| provider.models.first().map(|m| m.id.as_str()));
        return model.is_some_and(|model| provider.effective_capabilities(provider.models.iter().find(|entry| entry.id == model)).images.unwrap_or(false));
    }
    built_in_vision(kind, model)
}

/// Whether a built-in provider's model takes images: the catalog's word, else a guess from the
/// model's name.
fn built_in_vision(kind: &str, model: Option<&str>) -> bool {
    let default = default_model(kind);
    let model = model.map(str::trim).filter(|m| !m.is_empty()).unwrap_or(&default);
    if let Some(info) = models::find(kind, model) {
        return info.images;
    }
    let model = model.to_ascii_lowercase();
    match kind {
        "chatgpt" | "anthropic" | "grok" => true,
        "deepseek" => model.contains("vl") || model.contains("vision"),
        "opencode" | "opencode-go" => {
            model.contains("claude")
                || model.contains("gpt")
                || model.contains("gemini")
                || model.contains("grok")
                || model.contains("kimi")
                || model.contains("vision")
                || model.contains("qwen")
                || model.contains("glm")
        }
        _ => false,
    }
}

/// The first model the current validated catalog offers for a built-in provider.
pub fn default_model(kind: &str) -> String {
    models::default_model(kind).unwrap_or_default()
}

/// The model Auto-review runs on for bots of `kind`, and how much it thinks: the catalog's
/// `review` model for the provider, a small, fast one on the same account whatever the bot
/// itself runs, with thinking off where the model allows it and at its lowest effort where it
/// does not. A custom provider's is its first
/// model, the one the user put at the top, at the catalog's lowest level for a model the
/// catalog knows and the server's default for any other. Empty when `kind` has none.
pub fn review_model(app: &App, kind: &str) -> (String, Option<ThinkingLevel>) {
    if is_custom(kind) {
        let credentials = app.credentials.lock().unwrap();
        let Some(provider) = credentials.custom.get(kind) else { return (String::new(), None) };
        let Some(model) = provider.models.first() else { return (String::new(), None) };
        let thinking = provider.levels(model).first().copied();
        return (model.id.clone(), thinking);
    }
    let model = models::review_model(kind).unwrap_or_default();
    let thinking = models::find(kind, &model).and_then(|info| info.levels.first().copied()).unwrap_or(ThinkingLevel::Off);
    (model, Some(thinking))
}

/// A bot's thinking level as stored, or nothing for the provider's default.
pub fn thinking_level(bot: &crate::model::Bot) -> Option<ThinkingLevel> {
    bot.thinking.as_deref().and_then(|s| s.parse().ok())
}

pub fn provider_for(app: &Arc<App>, kind: &str, model: Option<&str>, thinking: Option<ThinkingLevel>) -> Result<Arc<dyn Provider>, String> {
    let model = model.filter(|m| !m.trim().is_empty()).map(str::to_string);
    match kind {
        kind if is_custom(kind) => {
            let credentials = app.credentials.lock().unwrap();
            let provider = credentials.custom.get(kind).ok_or_else(|| format!("{} is not connected", credentials.label(kind)))?;
            let model = model.or_else(|| provider.models.first().map(|m| m.id.clone())).ok_or_else(|| format!("{} has no models", provider.name))?;
            Ok(custom_provider(kind, provider, &model, thinking))
        }
        "deepseek" => {
            let key = app
                .credentials
                .lock()
                .unwrap()
                .deepseek
                .clone()
                .ok_or_else(|| "DeepSeek is not connected".to_string())?;
            let model = model.or_else(|| std::env::var("BEANS_DEEPSEEK_MODEL").ok()).unwrap_or_else(|| default_model(kind));
            // The Anthropic-compatible endpoint: the one with DeepSeek's server-side web search.
            let base_url = deepseek_anthropic_url(&key.base_url.clone().unwrap_or_else(deepseek_base_url));
            Ok(Arc::new(AnthropicProvider::deepseek(&key.api_key, Some(&model)).with_base_url(&base_url).with_thinking(thinking)))
        }
        "anthropic" => {
            let key = app
                .credentials
                .lock()
                .unwrap()
                .anthropic
                .clone()
                .ok_or_else(|| "Anthropic is not connected".to_string())?;
            let model = model.or_else(|| std::env::var("BEANS_ANTHROPIC_MODEL").ok()).unwrap_or_else(|| default_model(kind));
            let base_url = key.base_url.clone().unwrap_or_else(anthropic_base_url);
            Ok(Arc::new(AnthropicProvider::anthropic(&key.api_key, Some(&model)).with_base_url(&base_url).with_thinking(thinking)))
        }
        "opencode" => {
            let key = app
                .credentials
                .lock()
                .unwrap()
                .opencode
                .clone()
                .ok_or_else(|| "OpenCode Zen is not connected".to_string())?;
            let model = model.or_else(|| std::env::var("BEANS_OPENCODE_MODEL").ok()).unwrap_or_else(|| default_model(kind));
            let root = key.base_url.clone().or_else(|| env_url("BEANS_OPENCODE_BASE_URL")).unwrap_or_else(|| OPENCODE_BASE_URL.into());
            opencode_provider("opencode", &root, &key.api_key, &model, thinking)
        }
        "opencode-go" => {
            let key = app
                .credentials
                .lock()
                .unwrap()
                .opencode_go
                .clone()
                .ok_or_else(|| "OpenCode Go is not connected".to_string())?;
            let model = model.or_else(|| std::env::var("BEANS_OPENCODE_GO_MODEL").ok()).unwrap_or_else(|| default_model(kind));
            let root = key
                .base_url
                .clone()
                .or_else(|| env_url("BEANS_OPENCODE_GO_BASE_URL"))
                .unwrap_or_else(|| OPENCODE_GO_BASE_URL.into());
            opencode_provider("opencode-go", &root, &key.api_key, &model, thinking)
        }
        "chatgpt" => {
            if app.credentials.lock().unwrap().chatgpt.is_none() {
                return Err("ChatGPT is not connected".into());
            }
            let model = model.or_else(|| std::env::var("BEANS_CHATGPT_MODEL").ok()).unwrap_or_else(|| default_model(kind));
            Ok(Arc::new(ChatGptProvider::new(Arc::new(AppTokenSource(app.clone())), Some(&model)).with_thinking(thinking)))
        }
        "grok" => {
            if app.credentials.lock().unwrap().grok.is_none() {
                return Err("Grok is not connected".into());
            }
            let model = model.or_else(|| std::env::var("BEANS_GROK_MODEL").ok()).unwrap_or_else(|| default_model(kind));
            let mut provider = GrokProvider::new(Arc::new(AppGrokTokenSource(app.clone())), Some(&model)).with_thinking(thinking);
            if let Some(base_url) = env_url("BEANS_GROK_BASE_URL") {
                provider = provider.with_base_url(&base_url);
            }
            if let Some(issuer) = env_url("BEANS_GROK_ISSUER") {
                provider = provider.with_issuer(&issuer);
            }
            Ok(Arc::new(provider))
        }
        other => Err(format!("Unknown provider {other}")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenCodeWire {
    ChatCompletions,
    Messages,
    Responses,
    Unsupported,
}

/// OpenCode publishes the wire protocol beside every model, and the catalog carries it. A model
/// the catalog lacks goes by its family: Zen and Go differ for MiniMax and for Qwen3.8 Max,
/// which Zen serves on Chat Completions, while their GPT, Grok, and Muse families use Responses
/// and the rest of their Qwen family uses Messages.
fn opencode_wire(kind: &str, model: &str) -> OpenCodeWire {
    match models::find(kind, model).and_then(|info| info.wire) {
        Some(Wire::ChatCompletions) => return OpenCodeWire::ChatCompletions,
        Some(Wire::Messages) => return OpenCodeWire::Messages,
        Some(Wire::Responses) => return OpenCodeWire::Responses,
        None => {}
    }
    let model = model.to_ascii_lowercase();
    if (kind == "opencode" && model.starts_with("gemini-")) || model.starts_with("jev-") {
        return OpenCodeWire::Unsupported;
    }
    if model.starts_with("gpt-") || model.starts_with("grok-") || model.starts_with("muse-spark-") {
        return OpenCodeWire::Responses;
    }
    let qwen = model.starts_with("qwen") && !(kind == "opencode" && model == "qwen3.8-max");
    if qwen || (kind == "opencode-go" && model.starts_with("minimax-")) || model.starts_with("claude-") {
        return OpenCodeWire::Messages;
    }
    OpenCodeWire::ChatCompletions
}

/// Removes an optional `/v1` from a custom OpenCode root; each adapter adds the path its wire
/// protocol needs.
fn opencode_root(root: &str) -> &str {
    root.trim_end_matches('/').strip_suffix("/v1").unwrap_or_else(|| root.trim_end_matches('/'))
}

fn opencode_provider(
    kind: &str,
    root: &str,
    api_key: &str,
    model: &str,
    thinking: Option<ThinkingLevel>,
) -> Result<Arc<dyn Provider>, String> {
    let root = opencode_root(root);
    let provider: Arc<dyn Provider> = match opencode_wire(kind, model) {
        OpenCodeWire::ChatCompletions => {
            let mut provider = OpenAiCompatProvider::new(kind, &format!("{root}/v1"), api_key, model).with_thinking(thinking);
            provider.supports_images = built_in_vision(kind, Some(model));
            Arc::new(provider)
        }
        OpenCodeWire::Messages => {
            let mut provider = AnthropicProvider::new(kind, root, api_key, model).with_thinking(thinking);
            provider.supports_images = built_in_vision(kind, Some(model));
            provider.eager_tool_streaming = false;
            provider.max_tokens = 32_000;
            Arc::new(provider)
        }
        OpenCodeWire::Responses => {
            let mut provider = OpenAiResponsesProvider::new(kind, &format!("{root}/v1"), api_key, model).with_thinking(thinking);
            provider.supports_images = built_in_vision(kind, Some(model));
            Arc::new(provider)
        }
        OpenCodeWire::Unsupported => {
            return Err(format!("{model} uses an OpenCode endpoint Beans does not support"));
        }
    };
    Ok(opencode_headers(provider))
}

/// A bot's adapter for a custom provider: the wire protocol the user picked, at the root they
/// gave, with what is known about the model.
fn custom_provider(kind: &str, provider: &CustomProvider, model: &str, thinking: Option<ThinkingLevel>) -> Arc<dyn Provider> {
    let info = Arc::new(custom_model_info(kind, provider, model));
    let capabilities = provider.effective_capabilities(provider.models.iter().find(|entry| entry.id == model));
    match provider.api {
        CustomApi::ChatCompletions => {
            let mut adapter = OpenAiCompatProvider::new(kind, &provider.base_url, &provider.api_key, model).without_redirects().with_thinking(thinking);
            adapter.reasoning_effort_none = true;
            adapter.supports_tools = capabilities.tools.unwrap_or(true);
            adapter.supports_images = info.images;
            adapter.info = Some(info);
            // OpenAI's own field; servers such as Gemini's refuse a request with one they lack.
            adapter.prompt_cache_key = false;
            Arc::new(adapter)
        }
        CustomApi::Responses => {
            let mut adapter = OpenAiResponsesProvider::new(kind, &provider.base_url, &provider.api_key, model).without_redirects().with_thinking(thinking);
            adapter.supports_tools = capabilities.tools.unwrap_or(true);
            adapter.prompt_cache_key = false;
            adapter.supports_images = info.images;
            adapter.info = Some(info);
            Arc::new(adapter)
        }
        CustomApi::Messages => {
            let mut adapter = AnthropicProvider::new(kind, &provider.base_url, &provider.api_key, model).without_redirects().with_thinking(thinking);
            adapter.supports_tools = capabilities.tools.unwrap_or(true);
            adapter.supports_images = info.images;
            adapter.cache = false;
            adapter.max_tokens = if info.max_output > 0 { info.max_output.min(32_000) } else { 16_384 };
            adapter.info = Some(info);
            // Arguments streamed as they are generated are Anthropic's own extension.
            adapter.eager_tool_streaming = false;
            Arc::new(adapter)
        }
    }
}

/// Custom capabilities come only from this connection, never a similarly named catalog model.
fn custom_model_info(kind: &str, provider: &CustomProvider, model: &str) -> ModelInfo {
    let listed = provider.models.iter().find(|entry| entry.id == model);
    let capabilities = provider.effective_capabilities(listed);
    let thinking = if provider.api == CustomApi::Messages {
        match listed.and_then(|entry| entry.thinking_format.as_deref()) {
            Some("claude-adaptive") => ThinkingMode::Adaptive,
            _ => ThinkingMode::Budget,
        }
    } else { ThinkingMode::Effort };
    ModelInfo {
        id: model.to_string(),
        name: listed.and_then(|entry| entry.name.clone()).unwrap_or_else(|| model.to_string()),
        provider: kind.to_string(),
        context_window: capabilities.context_window.map(std::num::NonZeroU64::get).unwrap_or(0),
        max_output: listed.and_then(|entry| entry.max_output).unwrap_or(0),
        reasoning: listed.and_then(|entry| entry.reasoning).unwrap_or(false),
        images: capabilities.images.unwrap_or(false),
        rates: Rates { input: 0.0, output: 0.0, cache_read: 0.0, cache_write: 0.0 },
        tiers: Vec::new(),
        thinking,
        levels: listed.map(|entry| provider.levels(entry)).unwrap_or_default(),
        wire: None,
    }
}

/// DeepSeek's API root when the credential has none: `BEANS_DEEPSEEK_BASE_URL` (a proxy or a
/// test server) or DeepSeek itself.
fn deepseek_base_url() -> String {
    env_url("BEANS_DEEPSEEK_BASE_URL").unwrap_or_else(|| beans_agent::providers::openai_compat::DEEPSEEK_BASE_URL.to_string())
}

/// The Anthropic-compatible endpoint under a DeepSeek API root. A root given with its
/// `/anthropic` path already is used as is.
fn deepseek_anthropic_url(root: &str) -> String {
    let root = root.trim_end_matches('/');
    if root.ends_with("/anthropic") {
        root.to_string()
    } else {
        format!("{root}/anthropic")
    }
}

/// Anthropic's API root when the credential has none: `BEANS_ANTHROPIC_BASE_URL` or
/// Anthropic itself.
fn anthropic_base_url() -> String {
    env_url("BEANS_ANTHROPIC_BASE_URL").unwrap_or_else(|| ANTHROPIC_BASE_URL.to_string())
}

fn env_url(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.trim().is_empty()).map(|s| s.trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use crate::credentials::CustomModel;

    struct CaptureProvider(Arc<Mutex<Option<ModelRequest>>>);

    #[async_trait]
    impl Provider for CaptureProvider {
        fn provider_id(&self) -> &str {
            "capture"
        }

        fn model_id(&self) -> &str {
            "capture"
        }

        async fn stream(&self, request: ModelRequest, _cancel: CancellationToken) -> AssistantEventStream {
            *self.0.lock() = Some(request);
            Box::pin(futures::stream::empty())
        }
    }

    #[test]
    fn opencode_models_use_their_published_wire_protocols() {
        assert_eq!(opencode_wire("opencode", "gpt-6.1-sol"), OpenCodeWire::Responses);
        assert_eq!(opencode_wire("opencode", "muse-spark-1.3"), OpenCodeWire::Responses);
        assert_eq!(opencode_wire("opencode", "claude-sonnet-5-5"), OpenCodeWire::Messages);
        assert_eq!(opencode_wire("opencode", "deepseek-v4.1-flash"), OpenCodeWire::ChatCompletions);
        assert_eq!(opencode_wire("opencode", "minimax-m3"), OpenCodeWire::ChatCompletions);
        assert_eq!(opencode_wire("opencode", "qwen3.8-max"), OpenCodeWire::ChatCompletions);
        assert_eq!(opencode_wire("opencode", "gemini-3.8-flash"), OpenCodeWire::Unsupported);
        assert_eq!(opencode_wire("opencode-go", "minimax-m3"), OpenCodeWire::Messages);
        assert_eq!(opencode_wire("opencode-go", "qwen3.8-max"), OpenCodeWire::Messages);
        assert_eq!(opencode_wire("opencode-go", "grok-4.7"), OpenCodeWire::Responses);
        // Every model the catalog offers on OpenCode has a wire Beans speaks.
        for kind in ["opencode", "opencode-go"] {
            for model in models::for_provider(kind) {
                assert_ne!(opencode_wire(kind, &model.id), OpenCodeWire::Unsupported, "{kind}/{}", model.id);
            }
        }
    }

    #[test]
    fn every_opencode_route_takes_images_when_the_runner_sends_pixels() {
        for model in ["gpt-6.1-sol", "grok-4.7", "muse-spark-1.3", "claude-sonnet-5-5", "kimi-k3"] {
            assert!(opencode_provider("opencode", OPENCODE_BASE_URL, "k", model, None).unwrap().supports_images(), "{model}");
        }
        // A Responses model the catalog lacks, which the Runner sends no pixels, gets notes for
        // its tools' images too.
        assert!(!built_in_vision("opencode", Some("muse-spark-2")));
        assert!(!opencode_provider("opencode", OPENCODE_BASE_URL, "k", "muse-spark-2", None).unwrap().supports_images());
    }

    #[test]
    fn every_provider_has_a_default_and_roots_accept_v1() {
        for kind in ["deepseek", "anthropic", "chatgpt", "grok", "opencode", "opencode-go"] {
            assert!(models::find(kind, &default_model(kind)).is_some(), "{kind}");
        }
        assert_eq!(default_model("custom:lab"), "");
        // A model the catalog lacks goes by its family.
        assert_eq!(opencode_wire("opencode-go", "claude-unlisted-9"), OpenCodeWire::Messages);
        assert_eq!(opencode_root("https://opencode.ai/zen/v1/"), OPENCODE_BASE_URL);
        assert_eq!(opencode_root("https://opencode.ai/zen"), OPENCODE_BASE_URL);
    }

    struct ScratchApp(Arc<App>, std::path::PathBuf);

    impl Drop for ScratchApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    fn scratch_app() -> ScratchApp {
        let home = std::env::temp_dir().join(format!("beans-providers-{}", uuid::Uuid::new_v4()));
        ScratchApp(App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap(), home)
    }

    fn model(id: &str) -> CustomModel {
        CustomModel { id: id.into(), name: None, context_window: None, max_output: None, images: None, ..Default::default() }
    }

    fn add_custom(app: &App, kind: &str, api: CustomApi, models: Vec<CustomModel>) {
        let provider = CustomProvider { name: "Lab".into(), api, base_url: "http://127.0.0.1:9/v1".into(), api_key: String::new(), models, created_at: 1, integration: None, capabilities: None };
        app.credentials.lock().unwrap().custom.insert(kind.into(), provider);
    }

    #[test]
    fn auto_review_runs_a_small_model_that_thinks_least() {
        let scratch = scratch_app();
        let app = &scratch.0;
        for kind in ["deepseek", "anthropic", "chatgpt", "grok", "opencode", "opencode-go"] {
            assert!(models::find(kind, &review_model(app, kind).0).is_some(), "{kind}");
        }
        assert_eq!(review_model(app, "deepseek"), ("deepseek-flash".into(), Some(ThinkingLevel::Off)));
        assert_eq!(review_model(app, "anthropic"), ("claude-haiku-4-5".into(), Some(ThinkingLevel::Off)));
        assert_eq!(review_model(app, "chatgpt"), ("gpt-6-luna".into(), Some(ThinkingLevel::Low)));
        assert_eq!(review_model(app, "grok"), ("grok-4.7".into(), Some(ThinkingLevel::Low)));
        assert_eq!(review_model(app, "opencode-go"), ("deepseek-v4.1-flash".into(), Some(ThinkingLevel::Low)));
        // A custom provider reviews with its first model, at the server's default unless the
        // catalog knows the model.
        add_custom(app, "custom:lab", CustomApi::ChatCompletions, vec![model("qwen3:8b"), model("llama4")]);
        assert_eq!(review_model(app, "custom:lab"), ("qwen3:8b".into(), None));
        add_custom(app, "custom:proxy", CustomApi::Messages, vec![model("anthropic/claude-haiku-4-5")]);
        assert_eq!(review_model(app, "custom:proxy"), ("anthropic/claude-haiku-4-5".into(), None));
        assert_eq!(review_model(app, "custom:gone"), (String::new(), None));
    }

    #[test]
    fn durindoor_runtime_honors_declared_metadata_and_rejects_tool_incompatible_models() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let listed = CustomModel { context_window: Some(200_000), max_output: Some(32_000), images: Some(true), reasoning: Some(false), tools: Some(true), ..model("alias") };
        add_custom(app, "custom:gateway", CustomApi::ChatCompletions, vec![listed, CustomModel { tools: Some(false), ..model("no-tools") }]);
        app.credentials.lock().unwrap().custom.get_mut("custom:gateway").unwrap().integration = Some(crate::credentials::CustomIntegration::Durindoor);
        let provider = provider_for(app, "custom:gateway", None, Some(ThinkingLevel::High)).unwrap();
        let info = provider.model_info().unwrap();
        assert_eq!((info.context_window, info.max_output, info.images, info.reasoning), (200_000, 32_000, true, false));
        assert!(info.levels.is_empty());
        assert!(provider_for(app, "custom:gateway", Some("no-tools"), None).is_ok(), "tool-free models.ask and reviews can construct this adapter");
    }

    #[tokio::test]
    async fn connection_declarations_override_discovery_without_dropping_tools() {
        use futures::StreamExt;
        use beans_agent::AssistantEvent;
        let scratch = scratch_app();
        let app = &scratch.0;
        add_custom(app, "custom:declared", CustomApi::ChatCompletions, vec![CustomModel { images: Some(false), tools: Some(true), context_window: Some(1024), ..model("alias") }]);
        app.credentials.lock().unwrap().custom.get_mut("custom:declared").unwrap().capabilities = Some(crate::credentials::CustomCapabilities { context_window: std::num::NonZeroU64::new(8192), images: Some(true), tools: Some(false) });
        let adapter = provider_for(app, "custom:declared", Some("alias"), None).unwrap();
        assert!(supports_vision(app, "custom:declared", Some("alias")));
        assert_eq!(adapter.model_info().unwrap().context_window, 8192);
        let request = ModelRequest { system_prompt: String::new(), messages: Vec::new(), tools: vec![beans_agent::ToolSpec { name: "read".into(), description: "fixture".into(), parameters: serde_json::json!({"type":"object"}) }], cache_points: Vec::new(), max_tokens: None, options: Default::default() };
        let events: Vec<_> = adapter.stream(request, CancellationToken::new()).await.collect().await;
        assert!(matches!(&events[0], AssistantEvent::Error { message, aborted: false } if message.contains("does not support tools")));
    }

    #[test]
    fn custom_providers_run_their_first_model_with_what_is_known_of_it() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let listed = CustomModel { context_window: Some(32_768), images: Some(true), ..model("qwen3:8b") };
        add_custom(app, "custom:vision-lab", CustomApi::ChatCompletions, vec![listed, model("anthropic/claude-sonnet-5"), model("mystery")]);

        let provider = provider_for(app, "custom:vision-lab", None, None).unwrap();
        assert_eq!((provider.provider_id(), provider.model_id()), ("custom:vision-lab", "qwen3:8b"));
        assert_eq!(provider.model_info().map(|info| (info.context_window, info.images)), Some((32_768, true)));
        assert!(provider.supports_images());
        assert!(supports_vision(app, "custom:vision-lab", None));

        // A familiar model ID is not a capability declaration.
        let known = provider_for(app, "custom:vision-lab", Some("anthropic/claude-sonnet-5"), None).unwrap();
        let info = known.model_info().unwrap();
        assert_eq!((info.context_window, info.images, info.rates.input), (0, false, 0.0));

        // Nothing advertised: no window, text only, no guessed thinking.
        let unknown = provider_for(app, "custom:vision-lab", Some("mystery"), Some(ThinkingLevel::Max)).unwrap();
        let info = unknown.model_info().unwrap();
        assert_eq!((info.context_window, info.images, info.clamp_level(ThinkingLevel::Max)), (0, false, None));
        assert!(!supports_vision(app, "custom:vision-lab", Some("mystery")));

        // A running provider holds its own metadata even after the custom catalog changes.
        app.credentials.lock().unwrap().custom.get_mut("custom:vision-lab").unwrap().models[0].context_window = Some(65_536);
        assert_eq!(provider.model_info().unwrap().context_window, 32_768);
        let updated = provider_for(app, "custom:vision-lab", None, None).unwrap();
        assert_eq!(updated.model_info().unwrap().context_window, 65_536);

        app.credentials.lock().unwrap().custom.clear();
        assert_eq!(provider_for(app, "custom:vision-lab", None, None).err().unwrap(), "vision-lab is not connected");
    }

    #[tokio::test]
    async fn selected_aliases_keep_their_identity_and_metadata_on_each_wire() {
        use futures::StreamExt;
        use beans_agent::{AssistantEvent, LlmMessage, UserMessage};
        let cases = [
            (CustomApi::ChatCompletions, "/v1", "data: {\"choices\":[{\"delta\":{\"content\":\"fixture\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"),
            (CustomApi::Responses, "/v1", "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{}}}\n\n"),
            (CustomApi::Messages, "", "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"),
        ];
        for (api, suffix, fixture) in cases {
            let scratch = scratch_app();
            let app = &scratch.0;
            add_custom(app, "custom:aliases", api, vec![model("exact-alias"), CustomModel { images: Some(true), context_window: Some(12_345), ..model(" exact-alias ") }]);
            let adapter = provider_for(app, "custom:aliases", Some(" exact-alias "), None).unwrap();
            assert_eq!(adapter.model_id(), " exact-alias ");
            assert_eq!(adapter.model_info().unwrap().context_window, 12_345);
            assert!(supports_vision(app, "custom:aliases", Some(" exact-alias ")));
            assert!(!supports_vision(app, "custom:aliases", Some("exact-alias")));
            assert_eq!(provider_for(app, "custom:aliases", Some(" \t\n "), None).unwrap().model_id(), "exact-alias");
            let (root, server) = answer_once(fixture);
            app.credentials.lock().unwrap().custom.get_mut("custom:aliases").unwrap().base_url = format!("{root}{suffix}");
            let request = ModelRequest { system_prompt: String::new(), messages: vec![LlmMessage::User(UserMessage::text("hello"))], tools: Vec::new(), cache_points: Vec::new(), max_tokens: Some(32), options: Default::default() };
            let events: Vec<_> = provider_for(app, "custom:aliases", Some(" exact-alias "), None).unwrap().stream(request, CancellationToken::new()).await.collect().await;
            assert!(events.iter().any(|event| matches!(event, AssistantEvent::Done { .. })), "{events:?}");
            let seen = server.join().unwrap();
            let body: serde_json::Value = serde_json::from_str(seen.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(body["model"], " exact-alias ");
        }
    }

    #[tokio::test]
    async fn bot_updates_preserve_nonblank_model_ids_and_clear_blank_selections() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Fixture".into())).unwrap();
        let created = crate::api::dispatch(app, "bots.create", serde_json::json!({
            "name": "Alias bot", "runner_id": app.this_device_id().unwrap(), "provider": "custom:aliases", "model": "initial"
        })).await.unwrap();
        let id = created["bot"]["id"].as_str().unwrap();
        for selection in [" exact-alias ", "\tcombo/ALIAS\n", " \t\n "] {
            let updated = crate::api::dispatch(app, "bots.update", serde_json::json!({ "id": id, "model": selection })).await.unwrap();
            let expected = (!selection.trim().is_empty()).then_some(selection);
            assert_eq!(updated["bot"]["model"].as_str(), expected);
            assert_eq!(app.bot(id).unwrap().model.as_deref(), expected);
        }
    }

    /// Answers one model call with `body` as a stream and hands back the request it got.
    fn answer_once(body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0u8; 8192];
                let read = socket.read(&mut chunk).unwrap();
                assert!(read > 0, "request ended before its body");
                request.extend_from_slice(&chunk[..read]);
                let text = String::from_utf8_lossy(&request);
                if let Some((headers, body)) = text.split_once("\r\n\r\n") {
                    let length: usize = headers.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length: ").map(str::to_string)).unwrap().parse().unwrap();
                    if body.len() >= length { break; }
                }
            }
            let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            socket.write_all(reply.as_bytes()).unwrap();
            String::from_utf8(request).unwrap()
        });
        (root, server)
    }

    #[tokio::test]
    async fn a_server_that_takes_no_key_gets_no_auth_header() {
        use futures::StreamExt;
        let scratch = scratch_app();
        let app = &scratch.0;
        let request = || ModelRequest {
            system_prompt: String::new(),
            messages: vec![beans_agent::LlmMessage::User(beans_agent::UserMessage::text("hi"))],
            tools: Vec::new(),
            cache_points: Vec::new(),
            max_tokens: None,
            options: beans_agent::RequestOptions::default().with_session_id("chat-1"),
        };
        let cases = [
            (CustomApi::ChatCompletions, "/v1", "data: [DONE]\n\n", "post /v1/chat/completions "),
            (CustomApi::Messages, "", "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n", "post /v1/messages "),
            (CustomApi::Responses, "/v1", "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{}}}\n\n", "post /v1/responses "),
        ];
        for (api, path, body, line) in cases {
            let (root, server) = answer_once(body);
            let kind = format!("custom:keyless-{}", path.len());
            let provider = CustomProvider { name: "Keyless".into(), api, base_url: format!("{root}{path}"), api_key: String::new(), models: vec![model("m")], created_at: 1, integration: None, capabilities: None };
            app.credentials.lock().unwrap().custom.insert(kind.clone(), provider);
            let mut stream = provider_for(app, &kind, None, None).unwrap().stream(request(), CancellationToken::new()).await;
            while stream.next().await.is_some() {}
            let seen = server.join().unwrap().to_ascii_lowercase();
            assert!(seen.starts_with(line), "{seen}");
            assert!(!seen.contains("\r\nauthorization:") && !seen.contains("\r\nx-api-key:"), "{seen}");
            assert!(!seen.contains("prompt_cache_key"), "{seen}");
        }
    }


    #[tokio::test]
    async fn opencode_requests_identify_beans_and_carry_the_conversation() {
        let seen = Arc::new(Mutex::new(None));
        let provider = opencode_headers(Arc::new(CaptureProvider(seen.clone())));
        let request = ModelRequest {
            system_prompt: String::new(),
            messages: Vec::new(),
            tools: Vec::new(),
            cache_points: Vec::new(),
            max_tokens: None,
            options: beans_agent::RequestOptions::default().with_session_id("chat-1"),
        };
        let _ = provider.stream(request, CancellationToken::new()).await;
        let request = seen.lock().take().unwrap();
        assert_eq!(request.options.headers.get("User-Agent").map(String::as_str), Some(USER_AGENT));
        assert_eq!(request.options.headers.get("x-opencode-session").map(String::as_str), Some("chat-1"));
    }

    /// Fixtures use the official streamed shapes, without contacting a provider:
    /// https://platform.openai.com/docs/api-reference/chat/streaming
    /// https://platform.openai.com/docs/api-reference/responses-streaming
    /// https://platform.claude.com/docs/en/api/messages-streaming
    #[tokio::test]
    async fn compatible_metadata_drives_each_official_wire_without_id_guesses() {
        use futures::StreamExt;
        use beans_agent::{AssistantEvent, ContentPart, LlmMessage, UserMessage};
        let cases = [
            (CustomApi::ChatCompletions, "/v1", "/v1/chat/completions", "openai", "data: {\"choices\":[{\"delta\":{\"content\":\"fixture\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"),
            (CustomApi::Responses, "/v1", "/v1/responses", "openai", "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{}}}\n\n"),
            (CustomApi::Messages, "", "/v1/messages", "claude-adaptive", "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"),
            (CustomApi::Messages, "", "/v1/messages", "claude-budget", "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"),
        ];
        for (api, suffix, endpoint, format, fixture) in cases {
            for capability in [None, Some(false), Some(true)] {
                let (root, server) = answer_once(fixture);
                let listed = CustomModel { id: "claude-opus-5".into(), images: capability, reasoning: capability, tools: capability, thinking_format: Some(format.into()), thinking_can_disable: Some(true), max_output: Some(4096), ..Default::default() };
                let provider = CustomProvider { name: "Neutral".into(), api, base_url: format!("{root}{suffix}"), api_key: "fixture-key".into(), models: vec![listed], created_at: 1, integration: None, capabilities: None };
                let adapter = custom_provider("custom:neutral", &provider, "claude-opus-5", Some(ThinkingLevel::Medium));
                let request = ModelRequest {
                    system_prompt: "fixture system".into(),
                    messages: vec![LlmMessage::User(UserMessage { content: vec![ContentPart::text("fixture input"), ContentPart::Image { data: "AA==".into(), mime_type: "image/png".into() }], timestamp: 0 })],
                    tools: Vec::new(), cache_points: Vec::new(), max_tokens: Some(128),
                    options: Default::default(),
                };
                let events: Vec<_> = adapter.stream(request, CancellationToken::new()).await.collect().await;
                assert!(events.iter().any(|event| matches!(event, AssistantEvent::Done { stop_reason: beans_agent::StopReason::Stop, .. })), "{events:?}");
                assert!(!events.iter().any(|event| matches!(event, AssistantEvent::Error { .. })), "{events:?}");
                let seen = server.join().unwrap();
                let (headers, body) = seen.split_once("\r\n\r\n").unwrap();
                assert!(headers.starts_with(&format!("POST {endpoint} ")));
                let body: serde_json::Value = serde_json::from_str(body).unwrap();
                assert_eq!(body["model"], "claude-opus-5");
                assert_eq!(adapter.supports_images(), capability == Some(true));
                assert_eq!(body.to_string().contains("data:image/png;base64,AA==") || body.to_string().contains("\"data\":\"AA==\""), capability == Some(true));
                match api {
                    CustomApi::Messages => {
                        assert!(headers.contains("x-api-key: fixture-key"));
                        assert!(headers.contains("anthropic-version: 2023-06-01"));
                        assert!(!headers.contains("authorization:"));
                        assert_eq!(body.get("thinking").is_some(), capability == Some(true));
                        if capability == Some(true) {
                            if format == "claude-budget" {
                                assert_eq!(body["thinking"]["type"], "enabled");
                                assert_eq!(body["thinking"]["budget_tokens"], 3072);
                                assert_eq!(body["max_tokens"], 4096);
                                assert!(body.get("output_config").is_none());
                            } else {
                                assert_eq!(body["thinking"]["type"], "adaptive");
                                assert_eq!(body["output_config"]["effort"], "medium");
                            }
                        }
                        assert!(!body.to_string().contains("cache_control"));
                    }
                    CustomApi::ChatCompletions | CustomApi::Responses => {
                        assert!(headers.contains("authorization: Bearer fixture-key"));
                        assert!(!headers.contains("x-api-key:"));
                        if api == CustomApi::ChatCompletions {
                            assert_eq!(body.get("reasoning_effort").is_some(), capability == Some(true));
                            if capability == Some(true) { assert_eq!(body["reasoning_effort"], "medium"); }
                        } else {
                            assert_eq!(body.get("reasoning").is_some(), capability == Some(true));
                            if capability == Some(true) { assert_eq!(body["reasoning"]["effort"], "medium"); }
                        }
                    }
                }
                assert!(body.get("prompt_cache_key").is_none());
            }
            let provider = CustomProvider { name: "Neutral".into(), api, base_url: "http://127.0.0.1:9".into(), api_key: String::new(), models: vec![CustomModel { tools: Some(false), ..model("no-tools") }], created_at: 1, integration: None, capabilities: None };
            let request = ModelRequest { system_prompt: String::new(), messages: Vec::new(), tools: vec![beans_agent::ToolSpec { name: "read".into(), description: "fixture".into(), parameters: serde_json::json!({"type":"object"}) }], cache_points: Vec::new(), max_tokens: None, options: Default::default() };
            let events: Vec<_> = custom_provider("custom:neutral", &provider, "no-tools", None).stream(request, CancellationToken::new()).await.collect().await;
            assert!(matches!(&events[0], AssistantEvent::Error { message, aborted: false } if message.contains("does not support tools")));
        }
    }

    #[tokio::test]
    async fn compatible_inference_never_redirects_credentials_on_any_wire() {
        use futures::StreamExt;
        use std::io::{Read, Write};
        for api in CustomApi::ALL {
            let sink = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            sink.set_nonblocking(true).unwrap();
            let destination = format!("http://{}/capture",sink.local_addr().unwrap());
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let root = format!("http://{}",listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut socket,_) = listener.accept().unwrap();
                let mut request = [0u8;8192];
                socket.read(&mut request).unwrap();
                write!(socket,"HTTP/1.1 307 Temporary Redirect\r\nLocation: {destination}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            });
            let provider = CustomProvider { name:"Neutral".into(),api,base_url:root,api_key:"fixture-key".into(),models:vec![model("alias")],created_at:1,integration:None,capabilities:None };
            let request = ModelRequest { system_prompt:String::new(),messages:Vec::new(),tools:Vec::new(),cache_points:Vec::new(),max_tokens:Some(32),options:Default::default() };
            let events:Vec<_> = custom_provider("custom:neutral",&provider,"alias",None).stream(request,CancellationToken::new()).await.collect().await;
            server.join().unwrap();
            assert!(events.iter().any(|event| matches!(event,beans_agent::AssistantEvent::Error { message,aborted:false } if message.contains("307"))),"{events:?}");
            assert_eq!(sink.accept().unwrap_err().kind(),std::io::ErrorKind::WouldBlock);
        }
    }
}
