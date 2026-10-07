//! Saved generated appearance. Runtime state and animation frames never enter the roster.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum BotAvatarState { Idle, Thinking, Responding, Working, Waiting, Retry, Error }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Shape { Round, Organic, Boxy, Capsule, Nub, Cloud, Droplet, Hexagon, Sun, Triangle }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Expression { Idle, Happy, Sad, Mad, Surprised, Wink, Sleepy, Smug, Unsure, Scared, Love, Shy, Sick, Thinking }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Background { None, Square, Circle, Squircle }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Tone { Pastel, Pale, Mid, Deep, Bright, Ink }

/// Unknown fields read from a newer roster survive unrelated edits and re-encryption.
/// The local API only authors the frozen fields, through `BotLook::parse`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Palette {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eye: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Missing shape/hue/tone use the stable bot-id seed; expression is idle, background none,
/// and motion true. State appearances inherit omitted base fields, including palette channels.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BotAppearance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<Shape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<Expression>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Background>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hue: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<Palette>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BotLook {
    pub version: u32,
    pub base: BotAppearance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub states: Option<BTreeMap<BotAvatarState, BotAppearance>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

fn object_fields(value: &Value, allowed: &[&str], path: &str) -> Result<(), String> {
    let object = value.as_object().ok_or_else(|| format!("{path} must be an object"))?;
    for (key, value) in object {
        if !allowed.contains(&key.as_str()) || value.is_null() {
            return Err(format!("invalid {path}.{key}"));
        }
    }
    Ok(())
}

fn appearance_fields(value: &Value) -> Result<(), String> {
    object_fields(value, &["shape", "expression", "background", "hue", "tone", "palette", "motion"], "look.appearance")?;
    if let Some(palette) = value.get("palette") {
        object_fields(palette, &["head", "eye", "bg"], "look.palette")?;
    }
    Ok(())
}

impl BotLook {
    /// Strict authoring boundary: no unknown traits, CSS/SVG, functions, or null subfields.
    /// Persisted reads are separate so future fields are not silently discarded.
    pub fn parse(value: &Value) -> Result<Self, String> {
        object_fields(value, &["version", "base", "states"], "look")?;
        appearance_fields(value.get("base").ok_or("missing look.base")?)?;
        if let Some(states) = value.get("states") {
            let states = states.as_object().ok_or("look.states must be an object")?;
            for appearance in states.values() { appearance_fields(appearance)?; }
        }
        let look: Self = serde_json::from_value(value.clone()).map_err(|error| format!("invalid look: {error}"))?;
        look.validate()?;
        Ok(look)
    }

    /// Checks known persistence invariants without deleting fields received from newer readers.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 { return Err("look.version must be 1".into()); }
        self.base.validate()?;
        if let Some(states) = &self.states {
            for appearance in states.values() { appearance.validate()?; }
        }
        Ok(())
    }
}

impl BotAppearance {
    fn validate(&self) -> Result<(), String> {
        if self.hue.is_some_and(|hue| !hue.is_finite() || !(0.0..360.0).contains(&hue)) {
            return Err("look.hue must be finite and in [0, 360)".into());
        }
        if let Some(palette) = &self.palette {
            for color in [&palette.head, &palette.eye, &palette.bg].into_iter().flatten() {
                if color.len() != 7 || !color.starts_with('#')
                    || !color.as_bytes()[1..].iter().all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(byte)) {
                    return Err("look.palette colors must be uppercase #RRGGBB".into());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonfinite_hue_is_rejected_in_base_and_every_state() {
        for hue in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let appearance = BotAppearance { hue: Some(hue), ..Default::default() };
            let mut look = BotLook { version: 1, base: appearance.clone(), states: None, extra: BTreeMap::new() };
            assert!(look.validate().is_err());
            look.base = BotAppearance::default();
            for state in [BotAvatarState::Idle, BotAvatarState::Thinking, BotAvatarState::Responding, BotAvatarState::Working,
                BotAvatarState::Waiting, BotAvatarState::Retry, BotAvatarState::Error] {
                look.states = Some(BTreeMap::from([(state, appearance.clone())]));
                assert!(look.validate().is_err());
            }
        }
    }
}
