//! Model context, thinking, wire and rates from a versioned catalog bundled with every Device.
//! A newer validated catalog from the account's relay replaces it at runtime. Unknown models
//! run with no catalog window or cost. Each lookup owns its entry, so installs cannot invalidate
//! in-flight turns or grow process memory across refreshes.

mod catalog;
mod types;

pub use catalog::{install, parse, reset, updated, Catalog, BUNDLED};
pub use types::{Cost, ThinkingLevel, Usage};
use serde::Deserialize;
use std::sync::Arc;


/// Dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Rates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// Rates that apply once a request's input tokens exceed a size.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct CostTier {
    pub input_tokens_above: u64,
    pub rates: Rates,
}

/// How a model is asked to think.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingMode {
    /// `thinking: { type: "adaptive" }` with an `output_config.effort`; `Off` is
    /// `{ type: "disabled" }` when the model allows it.
    Adaptive,
    /// Adaptive thinking whose `Off` is `{ type: "between_tools" }` at the default effort, the
    /// lowest setting of a model that refuses `disabled` (Sonnet 5.5).
    AdaptiveBetweenTools,
    /// `thinking: { type: "enabled", budget_tokens }`; `Off` sends no thinking.
    Budget,
    /// A reasoning effort word (`reasoning_effort`, `reasoning.effort`). `Off`, on a model that
    /// lists it, is `thinking: { type: "disabled" }` on Chat Completions and the effort `none`
    /// on Responses.
    Effort,
}

impl ThinkingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Adaptive => "adaptive",
            Self::AdaptiveBetweenTools => "adaptive-between-tools",
            Self::Budget => "budget",
            Self::Effort => "effort",
        }
    }
}

impl std::str::FromStr for ThinkingMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        [Self::Adaptive, Self::AdaptiveBetweenTools, Self::Budget, Self::Effort]
            .into_iter().find(|mode| mode.as_str() == s)
            .ok_or_else(|| format!("Unknown thinking mode: {s}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire { ChatCompletions, Messages, Responses }

impl Wire {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat-completions",
            Self::Messages => "messages",
            Self::Responses => "responses",
        }
    }
}

impl std::str::FromStr for Wire {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        [Self::ChatCompletions, Self::Messages, Self::Responses]
            .into_iter().find(|wire| wire.as_str() == s)
            .ok_or_else(|| format!("Unknown wire protocol: {s}"))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub context_window: u64,
    pub max_output: u64,
    pub reasoning: bool,
    pub images: bool,
    pub rates: Rates,
    pub tiers: Vec<CostTier>,
    pub thinking: ThinkingMode,
    pub levels: Vec<ThinkingLevel>,
    pub wire: Option<Wire>,
}

impl ModelInfo {
    /// What a response cost, at the tier its input size lands in.
    pub fn cost_of(&self, usage: &Usage) -> Cost {
        let input_tokens = usage.input + usage.cache_read + usage.cache_write;
        let mut rates = self.rates;
        let mut matched = None;
        for tier in &self.tiers {
            if input_tokens > tier.input_tokens_above && matched.is_none_or(|t| tier.input_tokens_above > t) {
                rates = tier.rates;
                matched = Some(tier.input_tokens_above);
            }
        }
        let per = |rate: f64, tokens: u64| rate / 1_000_000.0 * tokens as f64;
        let input = per(rates.input, usage.input);
        let output = per(rates.output, usage.output);
        let cache_read = per(rates.cache_read, usage.cache_read);
        let cache_write = per(rates.cache_write, usage.cache_write);
        Cost { input, output, cache_read, cache_write, total: input + output + cache_read + cache_write }
    }

    /// The level this model runs at when asked for `level`: itself when supported, else the
    /// nearest supported level above it, else the highest the model has.
    pub fn clamp_level(&self, level: ThinkingLevel) -> Option<ThinkingLevel> {
        if self.levels.is_empty() {
            return None;
        }
        if self.levels.contains(&level) {
            return Some(level);
        }
        ThinkingLevel::ALL
            .iter()
            .copied()
            .find(|candidate| *candidate > level && self.levels.contains(candidate))
            .or_else(|| self.levels.last().copied())
    }
}

/// Coherent snapshot of current catalog; hold it across related lookups.
pub fn catalog() -> Arc<Catalog> { catalog::current() }

pub fn models() -> Vec<Arc<ModelInfo>> { catalog().models.clone() }

pub fn find(provider: &str, model: &str) -> Option<Arc<ModelInfo>> {
    let snapshot = catalog();
    snapshot.models.iter().find(|m| m.provider == provider && m.id == model)
        .or_else(|| snapshot.models.iter().find(|m| m.provider == provider && model.strip_prefix(&m.id).is_some_and(|rest| rest.starts_with('-'))))
        .cloned()
}

pub fn find_any(model: &str) -> Option<Arc<ModelInfo>> {
    let model = model.rsplit('/').next().unwrap_or(model);
    let snapshot = catalog();
    snapshot.models.iter().find(|m| m.id == model)
        .or_else(|| snapshot.models.iter().find(|m| model.strip_prefix(&m.id).and_then(|rest| rest.strip_prefix('-')).is_some_and(|date| !date.is_empty() && date.bytes().all(|c| c.is_ascii_digit()))))
        .cloned()
}

pub fn for_provider(provider: &str) -> Vec<Arc<ModelInfo>> {
    catalog().models.iter().filter(|m| m.provider == provider).cloned().collect()
}

pub fn default_model(provider: &str) -> Option<String> {
    catalog().models.iter().find(|m| m.provider == provider).map(|m| m.id.clone())
}

pub fn review_model(provider: &str) -> Option<String> { catalog().review.get(provider).cloned() }

#[cfg(test)]
mod tests {
    use super::*;
    use ThinkingLevel::{High, Low, Max, Minimal, Off, XHigh};

    #[test]
    fn cost_follows_the_rates_and_the_tier() {
        let sol = find("chatgpt", "gpt-6-sol").unwrap();
        let usage = Usage { input: 1_000_000, output: 100_000, cache_read: 0, cache_write: 0, ..Usage::default() };
        let cost = sol.cost_of(&usage);
        // Above 272k input tokens the whole request is at the long-context rate.
        assert!((cost.input - 4.0).abs() < 1e-9, "{cost:?}");
        assert!((cost.output - 1.5).abs() < 1e-9);
        assert!((cost.total - 5.5).abs() < 1e-9);

        let small = Usage { input: 1_000, output: 1_000, cache_read: 10_000, cache_write: 0, ..Usage::default() };
        let cost = sol.cost_of(&small);
        assert!((cost.input - 0.002).abs() < 1e-9);
        assert!((cost.cache_read - 0.002).abs() < 1e-9);
    }

    #[test]
    fn dated_ids_and_unknown_models() {
        assert_eq!(find("anthropic", "claude-haiku-4-5-20251001").as_ref().map(|m| m.id.as_str()), Some("claude-haiku-4-5"));
        assert!(find("anthropic", "claude-haiku-4").is_none());
        assert!(find("deepseek", "deepseek-chat").is_none());
        assert_eq!(for_provider("deepseek").first().map(|m| m.id.as_str()), Some("deepseek-flash"));
        assert_eq!(for_provider("chatgpt").first().map(|m| m.id.as_str()), Some("gpt-6.1-sol"));
        assert_eq!(for_provider("opencode").first().map(|m| m.id.as_str()), Some("deepseek-v4.1-flash"));
        assert_eq!(for_provider("opencode-go").first().map(|m| m.id.as_str()), Some("glm-5.3-flash"));
        assert_eq!(find("opencode-go", "qwen3.8-flash").map(|m| m.images), Some(true));
    }

    #[test]
    fn any_provider_knows_a_gateway_model() {
        assert_eq!(find_any("anthropic/claude-sonnet-5").as_ref().map(|m| m.id.as_str()), Some("claude-sonnet-5"));
        assert_eq!(find_any("claude-haiku-4-5-20251001").as_ref().map(|m| m.id.as_str()), Some("claude-haiku-4-5"));
        assert_eq!(find_any("moonshotai/kimi-k3").as_ref().map(|m| m.provider.as_str()), Some("opencode"));
        assert!(find_any("gpt-6-sol-mini").is_none(), "only a date extends an id");
        assert!(find_any("qwen3:8b").is_none());
    }

    #[test]
    fn levels_clamp_to_what_the_model_takes() {
        let fable = find("anthropic", "claude-fable-5-1").unwrap();
        assert_eq!(fable.clamp_level(Off), Some(Low));
        let opus = find("anthropic", "claude-opus-5-5").unwrap();
        assert_eq!(opus.clamp_level(Off), Some(Low));
        assert_eq!(opus.clamp_level(Minimal), Some(Low));
        assert_eq!(opus.clamp_level(Max), Some(Max));
        let haiku = find("anthropic", "claude-haiku-4-5").unwrap();
        assert_eq!(haiku.clamp_level(XHigh), Some(High));
        assert_eq!(haiku.clamp_level(Off), Some(Off));
        let sol = find("chatgpt", "gpt-6-sol").unwrap();
        assert_eq!(sol.clamp_level(Max), Some(Max));
        assert_eq!(sol.clamp_level(Off), Some(Low));
        assert_eq!(find("grok", "grok-4.7").unwrap().clamp_level(Max), Some(XHigh));
        // Zen can turn DeepSeek V4 Pro's thinking off; on Go it thinks at least at high.
        assert_eq!(find("opencode", "deepseek-v4-pro").unwrap().clamp_level(Off), Some(Off));
        assert_eq!(find("opencode-go", "deepseek-v4-pro").unwrap().clamp_level(Off), Some(High));
    }

    #[test]
    fn every_provider_has_a_review_model_it_lists() {
        for provider in ["deepseek", "anthropic", "chatgpt", "grok", "opencode", "opencode-go"] {
            let review = review_model(provider).unwrap_or_else(|| panic!("{provider} has no review model"));
            assert!(find(provider, &review).is_some(), "{provider}/{review}");
        }
    }
}
