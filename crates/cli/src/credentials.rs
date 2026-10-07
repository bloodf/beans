//! The account's provider credentials: API keys with an optional base URL, the ChatGPT and
//! Grok sign-ins, and the custom providers the user added. They travel as one `credentials`
//! blob under the account DEK, so every Device holds the same set in its private core folder;
//! a Runner builds its providers from them (`runner` feature).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::config::{self, Config};
use crate::model::{ProviderStatus, StatusModel};

#[cfg(feature = "provider-auth")]
pub use beans_provider_auth::{chatgpt::ChatGptTokens, grok::GrokTokens};
/// Builds without provider setup carry subscription tokens as opaque JSON.
#[cfg(not(feature = "provider-auth"))]
pub type ChatGptTokens = serde_json::Value;
#[cfg(not(feature = "provider-auth"))]
pub type GrokTokens = serde_json::Value;

/// The built-in providers. A custom provider's kind is `custom:` and a slug of its name.
pub const PROVIDER_KINDS: [&str; 6] = ["deepseek", "anthropic", "opencode", "opencode-go", "chatgpt", "grok"];

pub const CUSTOM_PREFIX: &str = "custom:";

/// Whether `kind` names a provider the user added.
pub fn is_custom(kind: &str) -> bool {
    kind.starts_with(CUSTOM_PREFIX)
}

/// The wire protocol a custom provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CustomApi {
    /// OpenAI-compatible Chat Completions, at `{base_url}/chat/completions`.
    ChatCompletions,
    /// OpenAI-compatible Responses, at `{base_url}/responses`.
    Responses,
    /// Anthropic-compatible Messages, at `{base_url}/v1/messages`.
    Messages,
}

impl CustomApi {
    pub const ALL: [CustomApi; 3] = [CustomApi::ChatCompletions, CustomApi::Responses, CustomApi::Messages];

    pub fn id(self) -> &'static str {
        match self {
            CustomApi::ChatCompletions => "chat-completions",
            CustomApi::Responses => "responses",
            CustomApi::Messages => "messages",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|api| api.id() == id)
    }
}


/// A model offered to a bot, including its display name and supported thinking levels.
#[derive(Debug, Clone, PartialEq)]
pub struct OfferedModel {
    pub id: String,
    pub name: String,
    pub levels: Vec<beans_models::ThinkingLevel>,
}


/// A model a custom provider offers, with what its server's model list said about it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CustomModel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output: Option<u64>,
    /// Whether it takes images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_can_disable: Option<bool>,
}

impl CustomModel {
    /// A sparse list updates only advertised facts, including explicit `false`.
    pub fn merge_metadata(&mut self, discovered: &Self) -> bool {
        let mut changed = false;
        macro_rules! take {
            ($($field:ident),+) => { $(if discovered.$field.is_some() && self.$field != discovered.$field {
                self.$field.clone_from(&discovered.$field);
                changed = true;
            })+ };
        }
        take!(name, context_window, max_output, images, reasoning, tools, thinking_format, thinking_can_disable);
        changed
    }
}

/// Integration identity is independent of the editable name and endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CustomIntegration {
    Durindoor,
}

impl CustomIntegration {
    pub fn parse(value: Option<&str>) -> Result<Option<Self>, String> {
        match value {
            None | Some("") => Ok(None),
            Some("durindoor") => Ok(Some(Self::Durindoor)),
            Some(_) => Err("Unknown provider integration".into()),
        }
    }
}

/// A server the user added that speaks one of the wire protocols Beans has: a gateway, another
/// vendor's API, or a model server on their own network.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomProvider {
    pub name: String,
    pub api: CustomApi,
    pub base_url: String,
    /// Empty for a server that takes no key.
    #[serde(default)]
    pub api_key: String,
    /// The models bots can pick, in the user's order; the first is the default.
    pub models: Vec<CustomModel>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration: Option<CustomIntegration>,
}

impl CustomProvider {
    /// Thinking controls require an explicit format supported by the selected wire.
    /// Legacy gateway formats retain their declared effort translation.
    pub fn levels(&self, model: &CustomModel) -> Vec<beans_models::ThinkingLevel> {
        use beans_models::ThinkingLevel::{Off, Minimal, Low, Medium, High, XHigh, Max};
        if model.reasoning != Some(true) { return Vec::new(); }
        if self.integration.is_none() {
            let supported = match self.api {
                CustomApi::ChatCompletions | CustomApi::Responses => model.thinking_format.as_deref() == Some("openai"),
                CustomApi::Messages => matches!(model.thinking_format.as_deref(), Some("claude-adaptive" | "claude-budget")),
            };
            if !supported { return Vec::new(); }
        }
        let mut levels = match model.thinking_format.as_deref() {
            Some("openai") => vec![Minimal, Low, Medium, High, XHigh],
            Some("claude-adaptive") => vec![Low, Medium, High, Max],
            Some("claude-budget") => vec![Low, Medium, High, XHigh, Max],
            Some("gemini-level") => vec![Minimal, Low, Medium, High],
            Some("gemini-budget" | "qwen" | "hunyuan" | "step") => vec![Low, Medium, High],
            Some("kimi") => vec![Low, Medium, High, Max],
            Some("deepseek") => vec![High],
            Some("opencode" | "ollama") => vec![Low, Medium, High, Max],
            Some("commandcode") => vec![Low, Medium, High, XHigh, Max],
            Some("openai-low-high-max") => vec![Low, High, Max],
            Some("zai" | "minimax") => vec![Low],
            _ => return Vec::new(),
        };
        if model.thinking_can_disable == Some(true) && !matches!(model.thinking_format.as_deref(), Some("gemini-level" | "commandcode" | "openai-low-high-max")) { levels.insert(0, Off); }
        levels
    }

    /// Preserve the user's model order and manually entered ids while extending the server's
    /// list. Refresh metadata for ids the server still advertises.
    pub fn merge_discovered_models(&mut self, discovered: Vec<CustomModel>) -> bool {
        let mut changed = false;
        for model in discovered {
            if let Some(existing) = self.models.iter_mut().find(|existing| existing.id == model.id) {
                changed |= existing.merge_metadata(&model);
            } else {
                self.models.push(model);
                changed = true;
            }
        }
        changed
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyCredential {
    pub api_key: String,
    /// The API root to call instead of the provider's own: a proxy or a compatible server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub connected_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Credentials {
    #[serde(default)]
    pub deepseek: Option<ApiKeyCredential>,
    #[serde(default)]
    pub anthropic: Option<ApiKeyCredential>,
    #[serde(default)]
    pub opencode: Option<ApiKeyCredential>,
    #[serde(default)]
    pub opencode_go: Option<ApiKeyCredential>,
    #[serde(default)]
    pub chatgpt: Option<ChatGptTokens>,
    #[serde(default)]
    pub grok: Option<GrokTokens>,
    /// The providers the user added, by kind.
    #[serde(default)]
    pub custom: BTreeMap<String, CustomProvider>,
    /// When each kind last changed on any Device (connected, tokens refreshed, disconnected),
    /// in seconds. Two Devices' sets merge kind by kind, the later change winning, so a
    /// disconnect is an entry here with no credential beside it.
    #[serde(default)]
    pub changed_at: BTreeMap<String, f64>,
}

/// What `Credentials::merge` found.
#[derive(Debug, Default, PartialEq)]
pub struct Merge {
    /// Kinds taken from the other set.
    pub taken: Vec<String>,
    /// This set has a change the other lacks, so the other side needs this one.
    pub is_ahead: bool,
}

impl Credentials {
    pub fn load(config: &Config) -> Self {
        let mut credentials: Credentials = config::read_json(&config.credentials_path()).unwrap_or_default();
        // A credential with no change time has never been merged: it counts from now, once.
        let unstamped: Vec<String> = credentials.connected_kinds().into_iter().filter(|kind| !credentials.changed_at.contains_key(kind)).collect();
        if !unstamped.is_empty() {
            for kind in unstamped {
                credentials.touch(&kind);
            }
            if let Err(error) = credentials.save(config) {
                tracing::error!(%error, "saving credentials");
            }
        }
        credentials
    }

    /// Marks `kind` as changed now, after a connect, a token refresh, or a disconnect.
    pub fn touch(&mut self, kind: &str) {
        self.changed_at.insert(kind.to_string(), config::now_secs());
    }

    pub fn is_empty(&self) -> bool {
        self.changed_at.is_empty()
    }

    /// Takes every kind `other` changed later than this set did.
    pub fn merge(&mut self, other: &Credentials) -> Merge {
        let mut merge = Merge::default();
        // A deleted custom provider stays in `changed_at`, so the deletion travels too.
        let custom: BTreeSet<&String> = self.changed_at.keys().chain(other.changed_at.keys()).filter(|kind| is_custom(kind)).collect();
        let kinds: Vec<String> = PROVIDER_KINDS.iter().map(|kind| kind.to_string()).chain(custom.into_iter().cloned()).collect();
        for kind in kinds {
            let (ours, theirs) = (self.changed_at.get(&kind).copied(), other.changed_at.get(&kind).copied());
            match (ours, theirs) {
                (ours, Some(theirs)) if ours.is_none_or(|ours| theirs > ours) => {
                    match kind.as_str() {
                        "deepseek" => self.deepseek = other.deepseek.clone(),
                        "anthropic" => self.anthropic = other.anthropic.clone(),
                        "opencode" => self.opencode = other.opencode.clone(),
                        "opencode-go" => self.opencode_go = other.opencode_go.clone(),
                        "chatgpt" => self.chatgpt = other.chatgpt.clone(),
                        "grok" => self.grok = other.grok.clone(),
                        _ => match other.custom.get(&kind) {
                            Some(provider) => {
                                self.custom.insert(kind.clone(), provider.clone());
                            }
                            None => {
                                self.custom.remove(&kind);
                            }
                        },
                    }
                    self.changed_at.insert(kind.clone(), theirs);
                    merge.taken.push(kind);
                }
                (Some(ours), theirs) if theirs.is_none_or(|theirs| ours > theirs) => merge.is_ahead = true,
                _ => {}
            }
        }
        merge
    }

    pub fn save(&self, config: &Config) -> anyhow::Result<()> {
        config::write_json_private(&config.credentials_path(), self)
    }

    pub(crate) fn api_key(&self, kind: &str) -> Option<&ApiKeyCredential> {
        match kind {
            "deepseek" => self.deepseek.as_ref(),
            "anthropic" => self.anthropic.as_ref(),
            "opencode" => self.opencode.as_ref(),
            "opencode-go" => self.opencode_go.as_ref(),
            _ => None,
        }
    }

    pub fn connected_kinds(&self) -> Vec<String> {
        self.statuses().into_iter().filter(|s| s.is_connected).map(|s| s.kind).collect()
    }

    /// Every kind a bot can run with: the built-in providers, then the custom ones.
    pub fn kinds(&self) -> Vec<String> {
        PROVIDER_KINDS.iter().map(|kind| kind.to_string()).chain(self.custom_kinds()).collect()
    }

    /// The custom providers' kinds, in the order the user added them.
    pub fn custom_kinds(&self) -> Vec<String> {
        let mut custom: Vec<(&String, &CustomProvider)> = self.custom.iter().collect();
        custom.sort_by(|a, b| a.1.created_at.cmp(&b.1.created_at).then_with(|| a.0.cmp(b.0)));
        custom.into_iter().map(|(kind, _)| kind.clone()).collect()
    }

    /// Built-in catalog or a custom provider's saved models, in default-first order.
    pub fn models(&self, kind: &str) -> Vec<OfferedModel> {
        if let Some(provider) = self.custom.get(kind) {
            return provider
                .models
                .iter()
                .map(|model| OfferedModel { id: model.id.clone(), name: model.name.clone().unwrap_or_else(|| model.id.clone()), levels: provider.levels(model) })
                .collect();
        }
        beans_models::for_provider(kind).into_iter().map(|model| OfferedModel { id: model.id.clone(), name: model.name.clone(), levels: model.levels.clone() }).collect()
    }

    /// The name people know a provider by: a custom provider's own, else the built-in's.
    pub fn label(&self, kind: &str) -> String {
        match self.custom.get(kind) {
            Some(provider) => provider.name.clone(),
            None => match kind {
                "deepseek" => "DeepSeek".into(),
                "anthropic" => "Anthropic".into(),
                "opencode" => "OpenCode Zen".into(),
                "opencode-go" => "OpenCode Go".into(),
                "chatgpt" => "ChatGPT".into(),
                "grok" => "Grok".into(),
                other => other.strip_prefix(CUSTOM_PREFIX).unwrap_or(other).to_string(),
            },
        }
    }

    pub fn statuses(&self) -> Vec<ProviderStatus> {
        let built_in = PROVIDER_KINDS.iter().map(|kind| {
            let detail = if *kind == "chatgpt" {
                self.chatgpt.as_ref().map(|t| chatgpt_email(t).unwrap_or_else(|| "Signed in".into()))
            } else if *kind == "grok" {
                self.grok.as_ref().map(|t| grok_email(t).unwrap_or_else(|| "Signed in".into()))
            } else {
                self.api_key(kind).map(|c| match &c.base_url {
                    Some(base_url) => format!("{} · {base_url}", mask_key(&c.api_key)),
                    None => mask_key(&c.api_key),
                })
            };
            ProviderStatus {
                kind: kind.to_string(),
                is_connected: detail.is_some(),
                detail: detail.unwrap_or_else(|| "Not connected".into()),
                base_url: self.api_key(kind).and_then(|c| c.base_url.clone()),
                ..Default::default()
            }
        });
        let custom = self.custom_kinds().into_iter().map(|kind| {
            let provider = &self.custom[&kind];
            let detail = match provider.api_key.trim() {
                "" => provider.base_url.clone(),
                key => format!("{} · {}", mask_key(key), provider.base_url),
            };
            ProviderStatus {
                kind,
                is_connected: true,
                detail,
                base_url: Some(provider.base_url.clone()),
                name: Some(provider.name.clone()),
                api: Some(provider.api),
                integration: provider.integration,
                models: provider.models.iter().map(|model| StatusModel { model: model.clone(), levels: provider.levels(model) }).collect(),
            }
        });
        built_in.chain(custom).collect()
    }
}

pub fn mask_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.len() <= 8 {
        return "••••".into();
    }
    format!("{}…{}", &trimmed[..3], &trimmed[trimmed.len() - 4..])
}


#[cfg(feature = "provider-auth")]
fn chatgpt_email(tokens: &ChatGptTokens) -> Option<String> {
    tokens.email.clone()
}

#[cfg(not(feature = "provider-auth"))]
fn chatgpt_email(tokens: &ChatGptTokens) -> Option<String> {
    tokens["email"].as_str().map(str::to_string)
}

#[cfg(feature = "provider-auth")]
fn grok_email(tokens: &GrokTokens) -> Option<String> {
    tokens.email.clone()
}

#[cfg(not(feature = "provider-auth"))]
fn grok_email(tokens: &GrokTokens) -> Option<String> {
    tokens["email"].as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(api_key: &str) -> Option<ApiKeyCredential> {
        Some(ApiKeyCredential { api_key: api_key.into(), base_url: None, connected_at: 0 })
    }

    #[test]
    fn durindoor_thinking_is_declared_and_unknown_formats_never_guess() {
        use beans_models::ThinkingLevel::{Off, Low, Medium, High, XHigh, Max};
        let mut gateway = custom("Renamed", 1);
        gateway.integration = Some(CustomIntegration::Durindoor);
        let mut model = CustomModel { id: "combo".into(), reasoning: Some(true), thinking_format: Some("claude-adaptive".into()), thinking_can_disable: Some(false), ..Default::default() };
        assert_eq!(gateway.levels(&model), [Low, Medium, High, Max]);
        model.thinking_can_disable = Some(true);
        assert_eq!(gateway.levels(&model), [Off, Low, Medium, High, Max]);
        model.id = "cx/gpt-6-sol".into();
        model.thinking_format = Some("openai".into());
        model.thinking_can_disable = Some(false);
        assert_eq!(gateway.levels(&model), [beans_models::ThinkingLevel::Minimal, Low, Medium, High, XHigh]);
        model.thinking_format = Some("future-format".into());
        assert!(gateway.levels(&model).is_empty());
        model.thinking_format = None;
        assert!(gateway.levels(&model).is_empty());
        model.reasoning = Some(false);
        model.thinking_format = Some("openai".into());
        assert!(gateway.levels(&model).is_empty());
        let legacy: CustomProvider = serde_json::from_value(serde_json::json!({"name":"Legacy","api":"chat-completions","base_url":"http://localhost/v1","models":[{"id":"unknown"}],"created_at":1})).unwrap();
        assert_eq!(legacy.integration, None);
        assert!(legacy.levels(&legacy.models[0]).is_empty());
    }

    #[test]
    fn merge_takes_the_later_change_of_each_kind() {
        let mut ours = Credentials { deepseek: key("old"), anthropic: key("ours"), ..Default::default() };
        ours.changed_at.insert("deepseek".into(), 1.0);
        ours.changed_at.insert("anthropic".into(), 5.0);
        let mut theirs = Credentials { deepseek: key("new"), ..Default::default() };
        theirs.changed_at.insert("deepseek".into(), 2.0);

        let merge = ours.merge(&theirs);
        assert_eq!(merge, Merge { taken: vec!["deepseek".into()], is_ahead: true });
        assert_eq!(ours.deepseek.as_ref().unwrap().api_key, "new");
        assert_eq!(ours.anthropic.as_ref().unwrap().api_key, "ours");

        // The same set again changes nothing and owes nothing.
        let again = ours.clone();
        assert_eq!(ours.merge(&again), Merge::default());
    }

    #[test]
    fn merge_carries_a_disconnect() {
        let mut ours = Credentials { deepseek: key("k"), ..Default::default() };
        ours.changed_at.insert("deepseek".into(), 1.0);
        let mut theirs = Credentials::default();
        theirs.changed_at.insert("deepseek".into(), 2.0);

        assert_eq!(ours.merge(&theirs).taken, vec!["deepseek".to_string()]);
        assert!(ours.deepseek.is_none());
        assert!(ours.connected_kinds().is_empty());
    }

    fn custom(name: &str, created_at: i64) -> CustomProvider {
        let models = vec![CustomModel { id: "m".into(), name: None, context_window: None, max_output: None, images: None, ..Default::default() }];
        CustomProvider { name: name.into(), api: CustomApi::ChatCompletions, base_url: "http://lab/v1".into(), api_key: String::new(), models, created_at, integration: None }
    }
    #[test]
    fn discovered_models_extend_user_order_without_changing_connection() {
        let mut provider = custom("Lab", 1);
        provider.api_key = "secret".into();
        let selected = provider.models[0].clone();
        let new = CustomModel { id: "new".into(), name: Some("New".into()), context_window: Some(128_000), max_output: None, images: Some(true), ..Default::default() };
        assert!(provider.merge_discovered_models(vec![new.clone(), selected.clone(), new.clone()]));
        assert_eq!(provider.models, [selected.clone(), new.clone()]);
        assert_eq!((provider.base_url.as_str(), provider.api_key.as_str()), ("http://lab/v1", "secret"));
        assert!(!provider.merge_discovered_models(vec![new.clone()]));
        assert_eq!(provider.models, [selected, new]);
    }

    #[test]
    fn later_user_edit_wins_against_stale_refreshed_catalog() {
        let mut local = Credentials::default();
        local.custom.insert("custom:lab".into(), custom("Lab", 1));
        local.changed_at.insert("custom:lab".into(), 3.0);
        let mut refreshed = local.clone();
        refreshed.custom.get_mut("custom:lab").unwrap().merge_discovered_models(vec![CustomModel { id: "new".into(), name: None, context_window: None, max_output: None, images: None, ..Default::default() }]);
        refreshed.changed_at.insert("custom:lab".into(), 4.0);
        local.custom.get_mut("custom:lab").unwrap().name = "User edit".into();
        local.changed_at.insert("custom:lab".into(), 5.0);
        assert_eq!(local.merge(&refreshed), Merge { taken: vec![], is_ahead: true });
        assert_eq!(local.custom["custom:lab"].name, "User edit");
        assert_eq!(local.custom["custom:lab"].models.len(), 1);
    }

    #[test]
    fn offered_models_follow_custom_order_without_guessed_levels() {
        let mut credentials = Credentials::default();
        let mut lab = custom("Lab", 1);
        lab.models.push(CustomModel { id: "anthropic/claude-opus-5".into(), name: Some("Opus".into()), context_window: None, max_output: None, images: None, ..Default::default() });
        credentials.custom.insert("custom:lab".into(), lab);
        let models = credentials.models("custom:lab");
        assert_eq!(models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(), ["m", "anthropic/claude-opus-5"]);
        assert_eq!(models[1].name, "Opus");
        assert!(models[1].levels.is_empty());
        assert_eq!(credentials.models("anthropic")[0].id, beans_models::for_provider("anthropic")[0].id);
        assert!(credentials.models("custom:gone").is_empty());
    }

    #[test]
    fn custom_providers_merge_one_by_one_and_deletions_travel() {
        let mut ours = Credentials::default();
        ours.custom.insert("custom:lab".into(), custom("Lab", 1));
        ours.changed_at.insert("custom:lab".into(), 1.0);
        ours.custom.insert("custom:mine".into(), custom("Mine", 2));
        ours.changed_at.insert("custom:mine".into(), 5.0);

        let mut theirs = Credentials::default();
        theirs.custom.insert("custom:router".into(), custom("Router", 3));
        theirs.changed_at.insert("custom:router".into(), 2.0);
        // Deleted on the other Device after this one added it.
        theirs.changed_at.insert("custom:lab".into(), 3.0);

        let merge = ours.merge(&theirs);
        assert_eq!(merge, Merge { taken: vec!["custom:lab".into(), "custom:router".into()], is_ahead: true });
        assert_eq!(ours.custom_kinds(), ["custom:mine", "custom:router"]);
        assert_eq!(ours.kinds().len(), PROVIDER_KINDS.len() + 2);
        assert_eq!(ours.label("custom:router"), "Router");
        assert_eq!(ours.label("custom:lab"), "lab");
        assert_eq!(ours.label("opencode-go"), "OpenCode Go");

        // Custom providers follow the built-in ones, in the order they were added.
        let statuses = ours.statuses();
        assert_eq!(statuses.len(), PROVIDER_KINDS.len() + 2);
        let mine = &statuses[PROVIDER_KINDS.len()];
        assert_eq!((mine.kind.as_str(), mine.is_connected, mine.detail.as_str()), ("custom:mine", true, "http://lab/v1"));
        assert_eq!(mine.models.len(), 1);
        assert!(mine.models[0].levels.is_empty());
        assert!(ours.connected_kinds().contains(&"custom:router".to_string()));
    }

    #[test]
    fn each_provider_offers_the_models_its_menu_lists() {
        let mut credentials = Credentials::default();
        let mut lab = custom("Lab", 1);
        lab.models.push(CustomModel { id: "anthropic/claude-opus-5".into(), name: Some("Opus 5".into()), context_window: None, max_output: None, images: None, ..Default::default() });
        credentials.custom.insert("custom:lab".into(), lab);

        // The catalog's, its default first.
        let anthropic = credentials.models("anthropic");
        assert_eq!(anthropic.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), beans_models::for_provider("anthropic").iter().map(|m| m.id.as_str()).collect::<Vec<_>>());
        // A custom provider's, named as its server names them, with the levels each takes.
        let lab = credentials.models("custom:lab");
        assert_eq!(lab.iter().map(|m| (m.id.as_str(), m.name.as_str())).collect::<Vec<_>>(), [("m", "m"), ("anthropic/claude-opus-5", "Opus 5")]);
        assert!(lab[1].levels.is_empty());
        assert!(credentials.models("custom:gone").is_empty());
    }

    #[test]
    fn opencode_credentials_merge_and_report_in_provider_order() {
        let mut ours = Credentials { opencode: key("zen-old"), ..Default::default() };
        ours.changed_at.insert("opencode".into(), 1.0);
        let mut theirs = Credentials { opencode: key("zen-new"), opencode_go: key("go-key"), ..Default::default() };
        theirs.changed_at.insert("opencode".into(), 2.0);
        theirs.changed_at.insert("opencode-go".into(), 2.0);

        assert_eq!(ours.merge(&theirs).taken, vec!["opencode".to_string(), "opencode-go".to_string()]);
        assert_eq!(ours.opencode.as_ref().unwrap().api_key, "zen-new");
        assert_eq!(ours.opencode_go.as_ref().unwrap().api_key, "go-key");
        assert_eq!(ours.statuses().into_iter().map(|status| status.kind).collect::<Vec<_>>(), PROVIDER_KINDS);
    }

    #[test]
    fn encrypted_legacy_credentials_sync_preserves_names_keys_order_and_discriminator() {
        let fixture = serde_json::json!({
            "custom": {
                "custom:legacy": {
                    "name":"User-selected historical name","api":"chat-completions",
                    "base_url":"https://gateway.example/v1","api_key":"fixture-key",
                    "created_at":12,"integration":"durindoor",
                    "models":[{"id":"guard/Combo","tools":false,"images":false,"context_window":200000},{"id":"manual"}]
                },
                "custom:generic": {
                    "name":"Unrecognized label","api":"messages","base_url":"https://proxy.example",
                    "api_key":"","created_at":13,"models":[{"id":"claude-opus-5"}]
                }
            },
            "changed_at":{"custom:legacy":14.0,"custom:generic":15.0}
        });
        let source: Credentials = serde_json::from_value(fixture).unwrap();
        let envelope = crate::crypto::encrypt_json(&[42;32],"credentials",&source).unwrap();
        let remote: Credentials = crate::crypto::decrypt_json(&[42;32],"credentials",&envelope).unwrap();
        let mut paired = Credentials::default();
        assert_eq!(paired.merge(&remote).taken, ["custom:generic","custom:legacy"]);
        assert_eq!(serde_json::to_value(&paired).unwrap(),serde_json::to_value(&source).unwrap());
        assert_eq!(paired.custom["custom:legacy"].models.iter().map(|m|m.id.as_str()).collect::<Vec<_>>(),["guard/Combo","manual"]);
        assert!(paired.models("custom:generic")[0].levels.is_empty());
    }
}
