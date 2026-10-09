//! Pure, closed #164 history wire types. No account capture, storage or execution authority.
//! Host adapters authenticate borrowed views and actors before passing them here.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_SAFE_UINT: u64 = 9_007_199_254_740_991;
pub const EVENT_BYTES: usize = 2048;
pub const RESPONSE_BYTES: usize = 512 * 1024;
pub const SCAN_LIMIT: usize = 201;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HistoryError {
    #[error("history_invalid_request")]
    Invalid,
    #[error("history_cursor_expired")]
    CursorExpired,
    #[error("history_unavailable")]
    Unavailable,
    #[error("history_conflict")]
    Conflict,
    #[error("history_capacity")]
    Capacity,
    #[error("history_unsupported")]
    Unsupported,
}
type Result<T> = std::result::Result<T, HistoryError>;
fn require(ok: bool) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(HistoryError::Invalid)
    }
}

macro_rules! number {
    ($name:ident, $min:expr, $max:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "u64", into = "u64")]
        pub struct $name(u64);
        impl TryFrom<u64> for $name {
            type Error = HistoryError;
            fn try_from(value: u64) -> Result<Self> {
                require(($min..=$max).contains(&value))?;
                Ok(Self(value))
            }
        }
        impl From<$name> for u64 {
            fn from(value: $name) -> u64 {
                value.0
            }
        }
        impl $name {
            pub fn get(self) -> u64 {
                self.0
            }
        }
    };
}
number!(SafeUInt, 0, MAX_SAFE_UINT);
number!(PositiveSafeUInt, 1, MAX_SAFE_UINT);
number!(PageLimit, 1, 200);
pub type Millis = SafeUInt;
pub type Seq = PositiveSafeUInt;
pub type RetentionRevision = PositiveSafeUInt;

macro_rules! closed_enum {
    ($name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name { $(#[serde(rename = $wire)] $variant),+ }
        impl $name { pub fn as_str(self) -> &'static str { match self { $(Self::$variant => $wire),+ } } }
    };
}
closed_enum!(ActorKind { Device => "device", System => "system", Automatic => "automatic", Unknown => "unknown" });
closed_enum!(Source { Human => "human", SavedRule => "saved_rule", Builtin => "builtin", AutoReview => "auto_review", PolicyLocal => "policy_local", PolicyObserved => "policy_observed", Recovery => "recovery" });
closed_enum!(Kind { Decision => "decision", ReviewObservation => "review_observation", PolicyChange => "policy_change", RuleChange => "rule_change", CoverageGap => "coverage_gap" });
closed_enum!(Outcome { Allow => "allow", Always => "always", Deny => "deny", Dismissed => "dismissed", Expired => "expired", Cancelled => "cancelled", Ask => "ask", Changed => "changed", Unavailable => "unavailable" });
closed_enum!(ActionClass { Plugin => "plugin", Shell => "shell", Proposal => "proposal", Install => "install", SignIn => "sign_in", AccountPolicy => "account_policy", BotCapability => "bot_capability", ReviewRule => "review_rule", Unknown => "unknown" });
closed_enum!(Reason { UserChoice => "user_choice", SavedRule => "saved_rule", BuiltinReadOnly => "builtin_read_only", AutomaticVerdict => "automatic_verdict", ReviewUnavailable => "review_unavailable", Timeout => "timeout", Cancelled => "cancelled", NewMessage => "new_message", RestartOrphan => "restart_orphan", PolicyEdit => "policy_edit", ObservedPolicy => "observed_policy", CoverageUnavailable => "coverage_unavailable" });
closed_enum!(PluginChange { Unchanged => "unchanged", Restricted => "restricted", Expanded => "expanded", Mixed => "mixed" });
closed_enum!(CursorMode { List => "list", Export => "export" });
closed_enum!(ReferenceCategory { Bot => "bot", Chat => "chat", Decision => "decision", Rule => "rule" });
closed_enum!(HumanChoice { Allow => "allow", Always => "always", Deny => "deny" });
closed_enum!(Verdict { Allow => "allow", Deny => "deny" });
closed_enum!(ReviewState { Ask => "ask", Unavailable => "unavailable" });
closed_enum!(WaitCause { Timeout => "timeout", Cancelled => "cancelled", NewMessage => "new_message" });
closed_enum!(OrphanChoice { Dismissed => "dismissed", Cancelled => "cancelled" });
closed_enum!(RuleOperation { Added => "added", Removed => "removed", Replaced => "replaced" });
closed_enum!(SettingOperation { Enabled => "enabled", Disabled => "disabled" });
closed_enum!(OccurrenceStage { Terminal => "terminal", Review => "review" });
closed_enum!(PolicyKind { Pause => "pause", Capabilities => "capabilities" });

pub fn validate_ref(value: &str) -> Result<()> {
    require(
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
    )
}
pub fn validate_uuid(value: &str) -> Result<()> {
    let id = uuid::Uuid::parse_str(value).map_err(|_| HistoryError::Invalid)?;
    require(id.hyphenated().to_string() == value)
}
pub fn validate_device_key(value: &str) -> Result<()> {
    require(value.len() == 43)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| HistoryError::Invalid)?;
    require(bytes.len() == 32 && URL_SAFE_NO_PAD.encode(&bytes) == value)
}
macro_rules! text {
    ($name:ident, $validator:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);
        impl TryFrom<String> for $name {
            type Error = HistoryError;
            fn try_from(value: String) -> Result<Self> {
                $validator(&value)?;
                Ok(Self(value))
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> String {
                value.0
            }
        }
        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}
text!(Ref, validate_ref);
text!(EventId, validate_uuid);
text!(PublicDeviceKey, validate_device_key);

/// Nullable is required: missing differs from explicitly null in the frozen schema.
fn nullable<'de, D, T>(d: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub kind: ActorKind,
    #[serde(deserialize_with = "nullable")]
    pub device_id: Option<PublicDeviceKey>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyRef {
    pub counter: SafeUInt,
    pub device_id: PublicDeviceKey,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyChange {
    #[serde(deserialize_with = "nullable")]
    pub paused: Option<bool>,
    #[serde(deserialize_with = "nullable")]
    pub shell: Option<bool>,
    #[serde(deserialize_with = "nullable")]
    pub write: Option<bool>,
    #[serde(deserialize_with = "nullable")]
    pub plugin_change: Option<PluginChange>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct References {
    #[serde(deserialize_with = "nullable")]
    pub bot: Option<Ref>,
    #[serde(deserialize_with = "nullable")]
    pub chat: Option<Ref>,
    #[serde(deserialize_with = "nullable")]
    pub decision: Option<Ref>,
    #[serde(deserialize_with = "nullable")]
    pub rule: Option<Ref>,
    #[serde(deserialize_with = "nullable")]
    pub policy: Option<PolicyRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryCode {
    Decision(ActionClass, DecisionSummary),
    ReviewAsk,
    ReviewUnavailable,
    PolicyChanged,
    RuleAdded,
    RuleRemoved,
    RuleReplaced,
    RuleEnabled,
    RuleDisabled,
    CoverageUnavailable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionSummary {
    Allowed,
    Always,
    Deny,
    Dismissed,
    Expired,
    Cancelled,
}
impl DecisionSummary {
    fn word(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Always => "always",
            Self::Deny => "deny",
            Self::Dismissed => "dismissed",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
        }
    }
    fn from_outcome(outcome: Outcome) -> Result<Self> {
        Ok(match outcome {
            Outcome::Allow => Self::Allowed,
            Outcome::Always => Self::Always,
            Outcome::Deny => Self::Deny,
            Outcome::Dismissed => Self::Dismissed,
            Outcome::Expired => Self::Expired,
            Outcome::Cancelled => Self::Cancelled,
            _ => return Err(HistoryError::Invalid),
        })
    }
}
impl Serialize for SummaryCode {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Decision(class, outcome) => {
                s.serialize_str(&format!("{}_action_{}", class.as_str(), outcome.word()))
            }
            _ => s.serialize_str(match self {
                Self::ReviewAsk => "review_ask",
                Self::ReviewUnavailable => "review_unavailable",
                Self::PolicyChanged => "policy_changed",
                Self::RuleAdded => "rule_added",
                Self::RuleRemoved => "rule_removed",
                Self::RuleReplaced => "rule_replaced",
                Self::RuleEnabled => "rule_enabled",
                Self::RuleDisabled => "rule_disabled",
                Self::CoverageUnavailable => "coverage_unavailable",
                Self::Decision(..) => unreachable!(),
            }),
        }
    }
}
impl<'de> Deserialize<'de> for SummaryCode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let value = String::deserialize(d)?;
        let fixed = match value.as_str() {
            "review_ask" => Some(Self::ReviewAsk),
            "review_unavailable" => Some(Self::ReviewUnavailable),
            "policy_changed" => Some(Self::PolicyChanged),
            "rule_added" => Some(Self::RuleAdded),
            "rule_removed" => Some(Self::RuleRemoved),
            "rule_replaced" => Some(Self::RuleReplaced),
            "rule_enabled" => Some(Self::RuleEnabled),
            "rule_disabled" => Some(Self::RuleDisabled),
            "coverage_unavailable" => Some(Self::CoverageUnavailable),
            _ => None,
        };
        if let Some(code) = fixed {
            return Ok(code);
        }
        if let Some((class, outcome)) = value.split_once("_action_") {
            let class = match class {
                "plugin" => ActionClass::Plugin,
                "shell" => ActionClass::Shell,
                "proposal" => ActionClass::Proposal,
                "install" => ActionClass::Install,
                "sign_in" => ActionClass::SignIn,
                "unknown" => ActionClass::Unknown,
                _ => return Err(serde::de::Error::custom("history_invalid_request")),
            };
            let outcome = match outcome {
                "allowed" => DecisionSummary::Allowed,
                "always" => DecisionSummary::Always,
                "deny" => DecisionSummary::Deny,
                "dismissed" => DecisionSummary::Dismissed,
                "expired" => DecisionSummary::Expired,
                "cancelled" => DecisionSummary::Cancelled,
                _ => return Err(serde::de::Error::custom("history_invalid_request")),
            };
            return Ok(Self::Decision(class, outcome));
        }
        Err(serde::de::Error::custom("history_invalid_request"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEventV1 {
    pub schema: u8,
    pub event_id: EventId,
    pub local_seq: Seq,
    pub recorded_at_ms: Millis,
    #[serde(deserialize_with = "nullable")]
    pub occurred_at_ms: Option<Millis>,
    pub observer_device_id: PublicDeviceKey,
    pub actor: Actor,
    pub source: Source,
    pub kind: Kind,
    pub outcome: Outcome,
    pub reason_code: Reason,
    pub action_class: ActionClass,
    pub refs: References,
    pub summary_code: SummaryCode,
    #[serde(deserialize_with = "nullable")]
    pub policy_change: Option<PolicyChange>,
}

/// Internal producer tag selects metadata; no raw body, arbitrary label or wire actor authority.
#[derive(Debug, Clone)]
pub enum ProducerPayload {
    HumanChoice(HumanChoice),
    SavedRulePermit,
    BuiltinReadOnlyPermit,
    AutomaticVerdict(Verdict),
    ReviewObservation(ReviewState),
    ClosedWait(WaitCause),
    OrphanClosure(OrphanChoice),
    LocalPause(bool),
    LocalCapabilities(PolicyChange),
    ObservedPause(bool),
    ObservedCapabilities(PolicyChange),
    RuleEdit(RuleOperation),
    ReviewSetting(SettingOperation),
    CoverageGap,
}
#[derive(Debug, Clone)]
pub struct EventContext {
    pub event_id: EventId,
    pub local_seq: Seq,
    pub recorded_at_ms: Millis,
    pub occurred_at_ms: Option<Millis>,
    pub observer_device_id: PublicDeviceKey,
    pub actor: Actor,
    pub action_class: ActionClass,
    pub refs: References,
}
impl ProducerPayload {
    fn metadata(&self) -> (Source, Kind, Outcome, Reason, Option<PolicyChange>) {
        use {Kind as K, Outcome as O, Reason as R, Source as S};
        let pause = |value| {
            Some(PolicyChange {
                paused: Some(value),
                shell: None,
                write: None,
                plugin_change: None,
            })
        };
        match self {
            Self::HumanChoice(choice) => (
                S::Human,
                K::Decision,
                match choice {
                    HumanChoice::Allow => O::Allow,
                    HumanChoice::Always => O::Always,
                    HumanChoice::Deny => O::Deny,
                },
                R::UserChoice,
                None,
            ),
            Self::SavedRulePermit => (S::SavedRule, K::Decision, O::Allow, R::SavedRule, None),
            Self::BuiltinReadOnlyPermit => {
                (S::Builtin, K::Decision, O::Allow, R::BuiltinReadOnly, None)
            }
            Self::AutomaticVerdict(choice) => (
                S::AutoReview,
                K::Decision,
                if *choice == Verdict::Allow {
                    O::Allow
                } else {
                    O::Deny
                },
                R::AutomaticVerdict,
                None,
            ),
            Self::ReviewObservation(state) => (
                S::AutoReview,
                K::ReviewObservation,
                if *state == ReviewState::Ask {
                    O::Ask
                } else {
                    O::Unavailable
                },
                R::ReviewUnavailable,
                None,
            ),
            Self::ClosedWait(cause) => {
                let (outcome, reason) = match cause {
                    WaitCause::Timeout => (O::Expired, R::Timeout),
                    WaitCause::Cancelled => (O::Cancelled, R::Cancelled),
                    WaitCause::NewMessage => (O::Dismissed, R::NewMessage),
                };
                (S::Human, K::Decision, outcome, reason, None)
            }
            Self::OrphanClosure(choice) => (
                S::Recovery,
                K::Decision,
                if *choice == OrphanChoice::Dismissed {
                    O::Dismissed
                } else {
                    O::Cancelled
                },
                R::RestartOrphan,
                None,
            ),
            Self::LocalPause(value) => (
                S::PolicyLocal,
                K::PolicyChange,
                O::Changed,
                R::PolicyEdit,
                pause(*value),
            ),
            Self::LocalCapabilities(delta) => (
                S::PolicyLocal,
                K::PolicyChange,
                O::Changed,
                R::PolicyEdit,
                Some(delta.clone()),
            ),
            Self::ObservedPause(value) => (
                S::PolicyObserved,
                K::PolicyChange,
                O::Changed,
                R::ObservedPolicy,
                pause(*value),
            ),
            Self::ObservedCapabilities(delta) => (
                S::PolicyObserved,
                K::PolicyChange,
                O::Changed,
                R::ObservedPolicy,
                Some(delta.clone()),
            ),
            Self::RuleEdit(_) => (S::Human, K::RuleChange, O::Changed, R::PolicyEdit, None),
            Self::ReviewSetting(_) => (
                S::PolicyLocal,
                K::RuleChange,
                O::Changed,
                R::PolicyEdit,
                None,
            ),
            Self::CoverageGap => (
                S::Recovery,
                K::CoverageGap,
                O::Unavailable,
                R::CoverageUnavailable,
                None,
            ),
        }
    }
    fn summary(&self, class: ActionClass, outcome: Outcome) -> Result<SummaryCode> {
        Ok(match self {
            Self::ReviewObservation(ReviewState::Ask) => SummaryCode::ReviewAsk,
            Self::ReviewObservation(ReviewState::Unavailable) => SummaryCode::ReviewUnavailable,
            Self::LocalPause(_)
            | Self::LocalCapabilities(_)
            | Self::ObservedPause(_)
            | Self::ObservedCapabilities(_) => SummaryCode::PolicyChanged,
            Self::RuleEdit(op) => match op {
                RuleOperation::Added => SummaryCode::RuleAdded,
                RuleOperation::Removed => SummaryCode::RuleRemoved,
                RuleOperation::Replaced => SummaryCode::RuleReplaced,
            },
            Self::ReviewSetting(op) => {
                if *op == SettingOperation::Enabled {
                    SummaryCode::RuleEnabled
                } else {
                    SummaryCode::RuleDisabled
                }
            }
            Self::CoverageGap => SummaryCode::CoverageUnavailable,
            _ => SummaryCode::Decision(class, DecisionSummary::from_outcome(outcome)?),
        })
    }
}
pub fn project_event_v1(context: EventContext, payload: ProducerPayload) -> Result<HistoryEventV1> {
    let (source, kind, outcome, reason_code, policy_change) = payload.metadata();
    let summary_code = payload.summary(context.action_class, outcome)?;
    let event = HistoryEventV1 {
        schema: 1,
        event_id: context.event_id,
        local_seq: context.local_seq,
        recorded_at_ms: context.recorded_at_ms,
        occurred_at_ms: context.occurred_at_ms,
        observer_device_id: context.observer_device_id,
        actor: context.actor,
        source,
        kind,
        outcome,
        reason_code,
        action_class: context.action_class,
        refs: context.refs,
        summary_code,
        policy_change,
    };
    validate_event_v1(&event)?;
    Ok(event)
}

pub fn validate_event_v1(e: &HistoryEventV1) -> Result<()> {
    use {ActorKind as A, Kind as K, Outcome as O, Reason as R, Source as S};
    require(e.schema == 1 && (e.actor.kind == A::Device) == e.actor.device_id.is_some())?;
    let refs = &e.refs;
    let no_action_refs = refs.chat.is_none() && refs.decision.is_none() && refs.rule.is_none();
    let no_refs = no_action_refs && refs.bot.is_none() && refs.policy.is_none();
    let decision_refs = refs.bot.is_some()
        && refs.chat.is_some()
        && refs.decision.is_some()
        && refs.policy.is_none();
    let payload = match e.kind {
        K::Decision => {
            require(
                decision_refs
                    && e.policy_change.is_none()
                    && matches!(
                        e.action_class,
                        ActionClass::Plugin
                            | ActionClass::Shell
                            | ActionClass::Proposal
                            | ActionClass::Install
                            | ActionClass::SignIn
                            | ActionClass::Unknown
                    ),
            )?;
            match (e.source, e.outcome, e.reason_code, e.actor.kind) {
                (S::Human, O::Allow | O::Deny | O::Always, R::UserChoice, A::Device) => {
                    require(
                        e.outcome != O::Always
                            || (e.action_class == ActionClass::Plugin && refs.rule.is_some()),
                    )?;
                    ProducerPayload::HumanChoice(match e.outcome {
                        O::Allow => HumanChoice::Allow,
                        O::Always => HumanChoice::Always,
                        _ => HumanChoice::Deny,
                    })
                }
                (S::SavedRule, O::Allow, R::SavedRule, A::Automatic) if refs.rule.is_some() => {
                    ProducerPayload::SavedRulePermit
                }
                (S::Builtin, O::Allow, R::BuiltinReadOnly, A::System)
                    if refs.rule.is_none()
                        && matches!(e.action_class, ActionClass::Plugin | ActionClass::Shell) =>
                {
                    ProducerPayload::BuiltinReadOnlyPermit
                }
                (S::AutoReview, O::Allow | O::Deny, R::AutomaticVerdict, A::Automatic)
                    if refs.rule.is_none() =>
                {
                    ProducerPayload::AutomaticVerdict(if e.outcome == O::Allow {
                        Verdict::Allow
                    } else {
                        Verdict::Deny
                    })
                }
                (S::Human, O::Expired, R::Timeout, A::System) if refs.rule.is_none() => {
                    ProducerPayload::ClosedWait(WaitCause::Timeout)
                }
                (S::Human, O::Cancelled, R::Cancelled, A::System) if refs.rule.is_none() => {
                    ProducerPayload::ClosedWait(WaitCause::Cancelled)
                }
                (S::Human, O::Dismissed, R::NewMessage, A::System) if refs.rule.is_none() => {
                    ProducerPayload::ClosedWait(WaitCause::NewMessage)
                }
                (S::Recovery, O::Dismissed | O::Cancelled, R::RestartOrphan, A::System)
                    if refs.rule.is_none() =>
                {
                    ProducerPayload::OrphanClosure(if e.outcome == O::Dismissed {
                        OrphanChoice::Dismissed
                    } else {
                        OrphanChoice::Cancelled
                    })
                }
                _ => return Err(HistoryError::Invalid),
            }
        }
        K::ReviewObservation => {
            require(
                decision_refs
                    && refs.rule.is_none()
                    && e.policy_change.is_none()
                    && e.source == S::AutoReview
                    && e.actor.kind == A::Automatic
                    && e.reason_code == R::ReviewUnavailable
                    && matches!(e.action_class, ActionClass::Plugin | ActionClass::Shell),
            )?;
            ProducerPayload::ReviewObservation(match e.outcome {
                O::Ask => ReviewState::Ask,
                O::Unavailable => ReviewState::Unavailable,
                _ => return Err(HistoryError::Invalid),
            })
        }
        K::PolicyChange => {
            require(e.outcome == O::Changed && refs.policy.is_some() && no_action_refs)?;
            let observed = match (e.source, e.reason_code, e.actor.kind) {
                (S::PolicyLocal, R::PolicyEdit, A::Device) => false,
                (S::PolicyObserved, R::ObservedPolicy, A::Unknown) => true,
                _ => return Err(HistoryError::Invalid),
            };
            let delta = e.policy_change.as_ref().ok_or(HistoryError::Invalid)?;
            match e.action_class {
                ActionClass::AccountPolicy => {
                    require(
                        refs.bot.is_none()
                            && delta.shell.is_none()
                            && delta.write.is_none()
                            && delta.plugin_change.is_none(),
                    )?;
                    let paused = delta.paused.ok_or(HistoryError::Invalid)?;
                    if observed {
                        ProducerPayload::ObservedPause(paused)
                    } else {
                        ProducerPayload::LocalPause(paused)
                    }
                }
                ActionClass::BotCapability => {
                    require(
                        refs.bot.is_some()
                            && delta.paused.is_none()
                            && (delta.shell.is_some()
                                || delta.write.is_some()
                                || matches!(
                                    delta.plugin_change,
                                    Some(
                                        PluginChange::Restricted
                                            | PluginChange::Expanded
                                            | PluginChange::Mixed
                                    )
                                )),
                    )?;
                    if observed {
                        ProducerPayload::ObservedCapabilities(delta.clone())
                    } else {
                        ProducerPayload::LocalCapabilities(delta.clone())
                    }
                }
                _ => return Err(HistoryError::Invalid),
            }
        }
        K::RuleChange => {
            require(
                e.outcome == O::Changed
                    && e.reason_code == R::PolicyEdit
                    && e.actor.kind == A::Device
                    && e.action_class == ActionClass::ReviewRule
                    && e.policy_change.is_none()
                    && refs.bot.is_none()
                    && refs.chat.is_none()
                    && refs.decision.is_none()
                    && refs.policy.is_none(),
            )?;
            match (e.source, e.summary_code) {
                (S::Human, SummaryCode::RuleAdded) if refs.rule.is_some() => {
                    ProducerPayload::RuleEdit(RuleOperation::Added)
                }
                (S::Human, SummaryCode::RuleRemoved) if refs.rule.is_some() => {
                    ProducerPayload::RuleEdit(RuleOperation::Removed)
                }
                (S::Human, SummaryCode::RuleReplaced) if refs.rule.is_some() => {
                    ProducerPayload::RuleEdit(RuleOperation::Replaced)
                }
                (S::PolicyLocal, SummaryCode::RuleEnabled) if refs.rule.is_none() => {
                    ProducerPayload::ReviewSetting(SettingOperation::Enabled)
                }
                (S::PolicyLocal, SummaryCode::RuleDisabled) if refs.rule.is_none() => {
                    ProducerPayload::ReviewSetting(SettingOperation::Disabled)
                }
                _ => return Err(HistoryError::Invalid),
            }
        }
        K::CoverageGap => {
            require(
                e.source == S::Recovery
                    && e.outcome == O::Unavailable
                    && e.reason_code == R::CoverageUnavailable
                    && e.actor.kind == A::System
                    && e.action_class == ActionClass::Unknown
                    && no_refs
                    && e.policy_change.is_none(),
            )?;
            ProducerPayload::CoverageGap
        }
    };
    require(e.summary_code == payload.summary(e.action_class, e.outcome)?)?;
    require(
        serde_json::to_vec(e)
            .map_err(|_| HistoryError::Invalid)?
            .len()
            <= EVENT_BYTES,
    )
}

/// Borrowed host inputs are not serializable and do not authenticate the host.
#[derive(Debug, Clone, Copy)]
pub enum EvidenceNamespaceInput<'a> {
    LocalTaskEpoch(&'a str),
    ImportedAccount(&'a str),
}
#[derive(Debug, Clone, Copy)]
pub struct EvidenceReadViewInput<'a> {
    pub account_id: &'a str,
    pub namespace: EvidenceNamespaceInput<'a>,
    pub incarnation: SafeUInt,
}
fn structural(value: &str) -> Result<()> {
    require(!value.is_empty() && value.len() <= 256)
}
fn frame(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u32).to_be_bytes());
    out.extend_from_slice(value);
}
fn namespace_bytes(view: EvidenceReadViewInput<'_>) -> Result<Vec<u8>> {
    structural(view.account_id)?;
    let (tag, value) = match view.namespace {
        EvidenceNamespaceInput::LocalTaskEpoch(value) => {
            validate_uuid(value)?;
            ("local-task-epoch", value)
        }
        EvidenceNamespaceInput::ImportedAccount(value) => {
            structural(value)?;
            require(value == view.account_id)?;
            ("imported-account", value)
        }
    };
    let mut bytes = Vec::with_capacity(600);
    bytes.extend_from_slice(b"beans.evidence.namespace.v1\0");
    frame(&mut bytes, view.account_id.as_bytes());
    frame(&mut bytes, tag.as_bytes());
    frame(&mut bytes, value.as_bytes());
    require(bytes.len() <= 1024)?;
    Ok(bytes)
}
// HMAC-SHA256 for an already supplied 32-byte purpose key; RFC 2104 fixed-key path.
fn mac(key: &[u8; 32], bytes: &[u8]) -> [u8; 32] {
    let mut inner_pad = [0x36; 64];
    let mut outer_pad = [0x5c; 64];
    for i in 0..32 {
        inner_pad[i] ^= key[i];
        outer_pad[i] ^= key[i];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(bytes);
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner.finalize());
    outer.finalize().into()
}
fn hex(bytes: &[u8; 32]) -> Ref {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 15) as usize] as char);
    }
    Ref(out)
}
fn namespaced_input(domain: &[u8], view: EvidenceReadViewInput<'_>) -> Result<Vec<u8>> {
    let namespace = namespace_bytes(view)?;
    let mut input = Vec::with_capacity(domain.len() + 4 + namespace.len() + 512);
    input.extend_from_slice(domain);
    frame(&mut input, &namespace);
    Ok(input)
}
pub fn history_namespace_v1(key: &[u8; 32], view: EvidenceReadViewInput<'_>) -> Result<Ref> {
    Ok(hex(&mac(
        key,
        &namespaced_input(b"beans.history.namespace.v1\0", view)?,
    )))
}
pub fn history_ref_v1(
    key: &[u8; 32],
    view: EvidenceReadViewInput<'_>,
    category: ReferenceCategory,
    id: &str,
) -> Result<Ref> {
    structural(id)?;
    let mut input = namespaced_input(b"beans.history.ref.v1\0", view)?;
    frame(&mut input, category.as_str().as_bytes());
    frame(&mut input, id.as_bytes());
    Ok(hex(&mac(key, &input)))
}
#[derive(Debug, Clone, Copy)]
pub struct InvocationOccurrenceInput<'a> {
    pub task_id: &'a str,
    pub owner_epoch: &'a str,
    pub execution_id: &'a str,
    pub invocation_id: &'a str,
    pub parent_id: Option<&'a str>,
    pub ordinal: u64,
    pub attempt_id: &'a str,
    pub receipt_id: &'a str,
    pub revision: u64,
    pub digest: &'a [u8; 32],
}
#[derive(Debug, Clone, Copy)]
pub enum HistoryOccurrenceInput<'a> {
    Invocation {
        binding: InvocationOccurrenceInput<'a>,
        stage: OccurrenceStage,
    },
    Review(InvocationOccurrenceInput<'a>),
    Card {
        card_id: &'a str,
        generation: &'a str,
        stage: OccurrenceStage,
    },
    Policy {
        counter: SafeUInt,
        device_key: &'a [u8; 32],
        subject: Option<&'a str>,
        kind: PolicyKind,
    },
}
pub fn history_occurrence_key_v1(
    key: &[u8; 32],
    view: EvidenceReadViewInput<'_>,
    occurrence: HistoryOccurrenceInput<'_>,
) -> Result<Ref> {
    let mut input = namespaced_input(b"beans.history.occurrence.v1\0", view)?;
    match occurrence {
        HistoryOccurrenceInput::Invocation { binding, .. }
        | HistoryOccurrenceInput::Review(binding) => {
            let review = matches!(occurrence, HistoryOccurrenceInput::Review(_));
            if !review {
                require(matches!(
                    view.namespace,
                    EvidenceNamespaceInput::LocalTaskEpoch(_)
                ))?;
            }
            frame(&mut input, if review { b"review" } else { b"invocation" });
            for value in [
                binding.task_id,
                binding.owner_epoch,
                binding.execution_id,
                binding.invocation_id,
            ] {
                structural(value)?;
                frame(&mut input, value.as_bytes());
            }
            if let Some(parent) = binding.parent_id {
                structural(parent)?;
            }
            frame(&mut input, binding.parent_id.unwrap_or("").as_bytes());
            input.extend_from_slice(&binding.ordinal.to_be_bytes());
            for value in [binding.attempt_id, binding.receipt_id] {
                structural(value)?;
                frame(&mut input, value.as_bytes());
            }
            input.extend_from_slice(&binding.revision.to_be_bytes());
            input.extend_from_slice(binding.digest);
            let stage = match occurrence {
                HistoryOccurrenceInput::Invocation { stage, .. } => stage,
                _ => OccurrenceStage::Review,
            };
            frame(&mut input, stage.as_str().as_bytes());
        }
        HistoryOccurrenceInput::Card {
            card_id,
            generation,
            stage,
        } => {
            structural(card_id)?;
            structural(generation)?;
            frame(&mut input, b"card");
            frame(&mut input, card_id.as_bytes());
            frame(&mut input, generation.as_bytes());
            frame(&mut input, stage.as_str().as_bytes());
        }
        HistoryOccurrenceInput::Policy {
            counter,
            device_key,
            subject,
            kind,
        } => {
            if let Some(subject) = subject {
                structural(subject)?;
            }
            frame(&mut input, b"policy");
            input.extend_from_slice(&counter.get().to_be_bytes());
            input.extend_from_slice(device_key);
            frame(&mut input, subject.unwrap_or("").as_bytes());
            frame(&mut input, kind.as_str().as_bytes());
        }
    }
    require(input.len() <= 2048)?;
    Ok(hex(&mac(key, &input)))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryCursorV1 {
    pub v: u8,
    pub namespace: Ref,
    pub incarnation: SafeUInt,
    pub retention_revision: RetentionRevision,
    pub through_seq: SafeUInt,
    pub after_seq: SafeUInt,
    #[serde(deserialize_with = "nullable")]
    pub kind: Option<Kind>,
    pub mode: CursorMode,
}
fn cursor_payload(cursor: &HistoryCursorV1) -> Result<Vec<u8>> {
    require(cursor.v == 1 && cursor.after_seq <= cursor.through_seq)?;
    let bytes = serde_json::to_vec(cursor).map_err(|_| HistoryError::Invalid)?;
    require(bytes.len() <= 1024)?;
    Ok(bytes)
}
fn cursor_mac(key: &[u8; 32], payload: &[u8]) -> [u8; 32] {
    let mut input = Vec::with_capacity(28 + payload.len());
    input.extend_from_slice(b"beans.history.cursor.v1\0");
    frame(&mut input, payload);
    mac(key, &input)
}
fn constant_time_equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    // Fixed iteration count; black_box prevents replacing this with an early-exit comparison.
    let mut difference = 0u8;
    for i in 0..32 {
        difference |= std::hint::black_box(left[i] ^ right[i]);
    }
    std::hint::black_box(difference) == 0
}
pub fn encode_history_cursor_v1(
    key: &[u8; 32],
    cursor: &HistoryCursorV1,
    view: EvidenceReadViewInput<'_>,
    revision: RetentionRevision,
    mode: CursorMode,
) -> Result<String> {
    check_cursor_context(key, cursor, view, revision, mode)?;
    let payload = cursor_payload(cursor)?;
    Ok(format!(
        "h164c1.{}.{}",
        URL_SAFE_NO_PAD.encode(&payload),
        URL_SAFE_NO_PAD.encode(cursor_mac(key, &payload))
    ))
}
fn check_cursor_context(
    key: &[u8; 32],
    cursor: &HistoryCursorV1,
    view: EvidenceReadViewInput<'_>,
    revision: RetentionRevision,
    mode: CursorMode,
) -> Result<()> {
    if cursor.namespace != history_namespace_v1(key, view)?
        || cursor.incarnation != view.incarnation
        || cursor.retention_revision != revision
        || cursor.mode != mode
    {
        return Err(HistoryError::CursorExpired);
    }
    Ok(())
}
pub fn decode_history_cursor_v1(
    key: &[u8; 32],
    encoded: &str,
    view: EvidenceReadViewInput<'_>,
    revision: RetentionRevision,
    mode: CursorMode,
) -> Result<HistoryCursorV1> {
    require(encoded.len() <= 2048)?;
    let mut parts = encoded.split('.');
    require(parts.next() == Some("h164c1"))?;
    let payload_part = parts.next().ok_or(HistoryError::Invalid)?;
    let tag_part = parts.next().ok_or(HistoryError::Invalid)?;
    require(parts.next().is_none() && payload_part.len() <= 1366 && tag_part.len() == 43)?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload_part)
        .map_err(|_| HistoryError::Invalid)?;
    let tag: [u8; 32] = URL_SAFE_NO_PAD
        .decode(tag_part)
        .map_err(|_| HistoryError::Invalid)?
        .try_into()
        .map_err(|_| HistoryError::Invalid)?;
    require(
        payload.len() <= 1024
            && URL_SAFE_NO_PAD.encode(&payload) == payload_part
            && URL_SAFE_NO_PAD.encode(tag) == tag_part,
    )?;
    require(constant_time_equal(&tag, &cursor_mac(key, &payload)))?;
    let cursor: HistoryCursorV1 =
        serde_json::from_slice(&payload).map_err(|_| HistoryError::Invalid)?;
    require(cursor_payload(&cursor)? == payload)?;
    check_cursor_context(key, &cursor, view, revision, mode)?;
    Ok(cursor)
}

/// Bounded production event decoder; callers never expose unchecked deserialized events.
pub fn decode_event_v1(bytes: &[u8]) -> Result<HistoryEventV1> {
    let event = decode_closed(bytes, EVENT_BYTES)?;
    validate_event_v1(&event)?;
    Ok(event)
}
pub fn encode_event_v1(event: &HistoryEventV1) -> Result<Vec<u8>> {
    validate_event_v1(event)?;
    serde_json::to_vec(event).map_err(|_| HistoryError::Invalid)
}
fn decode_closed<T: DeserializeOwned>(bytes: &[u8], cap: usize) -> Result<T> {
    require(bytes.len() <= cap)?;
    serde_json::from_slice(bytes).map_err(|_| HistoryError::Invalid)
}

/// Optional request keys may be absent, but may not be explicitly null.
fn present<'de, D, T>(d: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(d).map(Some)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "bool", into = "bool")]
pub struct Confirmed;
impl TryFrom<bool> for Confirmed {
    type Error = HistoryError;
    fn try_from(value: bool) -> Result<Self> {
        require(value)?;
        Ok(Self)
    }
}
impl From<Confirmed> for bool {
    fn from(_: Confirmed) -> bool {
        true
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub enum Days {
    Seven,
    Thirty,
    Ninety,
}
impl TryFrom<u64> for Days {
    type Error = HistoryError;
    fn try_from(value: u64) -> Result<Self> {
        match value {
            7 => Ok(Self::Seven),
            30 => Ok(Self::Thirty),
            90 => Ok(Self::Ninety),
            _ => Err(HistoryError::Invalid),
        }
    }
}
impl From<Days> for u64 {
    fn from(value: Days) -> u64 {
        match value {
            Days::Seven => 7,
            Days::Thirty => 30,
            Days::Ninety => 90,
        }
    }
}
fn validate_cursor_text(value: &str) -> Result<()> {
    require(value.len() <= 2048 && value.starts_with("h164c1."))
}
text!(Cursor, validate_cursor_text);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum HistoryListRequest {
    First {
        #[serde(skip_serializing_if = "Option::is_none")]
        after_seq: Option<SafeUInt>,
        #[serde(skip_serializing_if = "Option::is_none")]
        through_seq: Option<SafeUInt>,
        #[serde(skip_serializing_if = "Option::is_none")]
        limit: Option<PageLimit>,
        #[serde(skip_serializing_if = "Option::is_none")]
        kind: Option<Kind>,
    },
    Continuation {
        cursor: Cursor,
        #[serde(skip_serializing_if = "Option::is_none")]
        limit: Option<PageLimit>,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListRequestWire {
    #[serde(default, deserialize_with = "present")]
    after_seq: Option<SafeUInt>,
    #[serde(default, deserialize_with = "present")]
    through_seq: Option<SafeUInt>,
    #[serde(default, deserialize_with = "present")]
    limit: Option<PageLimit>,
    #[serde(default, deserialize_with = "present")]
    kind: Option<Kind>,
    #[serde(default, deserialize_with = "present")]
    cursor: Option<Cursor>,
}
impl ListRequestWire {
    fn into_request(self) -> Result<HistoryListRequest> {
        if let Some(cursor) = self.cursor {
            require(self.after_seq.is_none() && self.through_seq.is_none() && self.kind.is_none())?;
            Ok(HistoryListRequest::Continuation {
                cursor,
                limit: self.limit,
            })
        } else {
            if let Some(through) = self.through_seq {
                require(self.after_seq.map_or(0, SafeUInt::get) <= through.get())?;
            }
            Ok(HistoryListRequest::First {
                after_seq: self.after_seq,
                through_seq: self.through_seq,
                limit: self.limit,
                kind: self.kind,
            })
        }
    }
}
impl<'de> Deserialize<'de> for HistoryListRequest {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        ListRequestWire::deserialize(d)?
            .into_request()
            .map_err(serde::de::Error::custom)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryExportRequest {
    #[serde(flatten)]
    pub page: HistoryListRequest,
    pub confirm: Confirmed,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportRequestWire {
    #[serde(default, deserialize_with = "present")]
    after_seq: Option<SafeUInt>,
    #[serde(default, deserialize_with = "present")]
    through_seq: Option<SafeUInt>,
    #[serde(default, deserialize_with = "present")]
    limit: Option<PageLimit>,
    #[serde(default, deserialize_with = "present")]
    kind: Option<Kind>,
    #[serde(default, deserialize_with = "present")]
    cursor: Option<Cursor>,
    confirm: Confirmed,
}
impl<'de> Deserialize<'de> for HistoryExportRequest {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let w = ExportRequestWire::deserialize(d)?;
        let page = ListRequestWire {
            after_seq: w.after_seq,
            through_seq: w.through_seq,
            limit: w.limit,
            kind: w.kind,
            cursor: w.cursor,
        }
        .into_request()
        .map_err(serde::de::Error::custom)?;
        Ok(Self {
            page,
            confirm: w.confirm,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPage {
    pub after_seq: SafeUInt,
    pub through_seq: SafeUInt,
    pub limit: PageLimit,
    pub kind: Option<Kind>,
}
impl HistoryListRequest {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        decode_closed(bytes, 4096)
    }
    /// Pure boundary resolution; watermark comes from the later authenticated store adapter.
    pub fn resolve(
        &self,
        key: &[u8; 32],
        view: EvidenceReadViewInput<'_>,
        revision: RetentionRevision,
        current: SafeUInt,
        mode: CursorMode,
    ) -> Result<ResolvedPage> {
        let (after_seq, through_seq, limit, kind) = match self {
            Self::First {
                after_seq,
                through_seq,
                limit,
                kind,
            } => (*after_seq, *through_seq, *limit, *kind),
            Self::Continuation { cursor, limit } => {
                let payload = decode_history_cursor_v1(key, cursor.as_str(), view, revision, mode)?;
                (
                    Some(payload.after_seq),
                    Some(payload.through_seq),
                    *limit,
                    payload.kind,
                )
            }
        };
        let after_seq = after_seq.unwrap_or(SafeUInt(0));
        let through_seq = through_seq.unwrap_or(current);
        require(after_seq <= through_seq && through_seq <= current)?;
        Ok(ResolvedPage {
            after_seq,
            through_seq,
            limit: limit.unwrap_or(PageLimit(50)),
            kind,
        })
    }
}
impl HistoryExportRequest {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        decode_closed(bytes, 4096)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRetentionGetRequest {}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRetentionSetRequest {
    pub days: Days,
    pub confirm: Confirmed,
    pub expected_revision: RetentionRevision,
}
pub enum HistoryRetentionRequest {
    Get(HistoryRetentionGetRequest),
    Set(HistoryRetentionSetRequest),
}
impl HistoryRetentionRequest {
    pub fn decode_get(bytes: &[u8]) -> Result<Self> {
        decode_closed(bytes, 4096).map(Self::Get)
    }
    pub fn decode_set(bytes: &[u8]) -> Result<Self> {
        decode_closed(bytes, 4096).map(Self::Set)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub since_ms: Millis,
    pub local_only: Confirmed,
    pub unavailable: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retention {
    pub days: Days,
    #[serde(deserialize_with = "nullable")]
    pub oldest_available_seq: Option<Seq>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryListResult {
    pub schema: u8,
    pub events: Vec<HistoryEventV1>,
    pub through_seq: SafeUInt,
    #[serde(deserialize_with = "nullable")]
    pub next_after_seq: Option<SafeUInt>,
    #[serde(deserialize_with = "nullable")]
    pub cursor: Option<Cursor>,
    pub coverage: Coverage,
    pub retention: Retention,
}
fn bounded_encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value).map_err(|_| HistoryError::Invalid)?;
    require(bytes.len() <= RESPONSE_BYTES)?;
    Ok(bytes)
}
/// Host-supplied scan facts and borrowed authentication inputs, never wire authority.
pub struct PageValidation<'a> {
    pub key: &'a [u8; 32],
    pub view: EvidenceReadViewInput<'a>,
    pub revision: RetentionRevision,
    pub scanned: usize,
    pub last_scanned: SafeUInt,
}
fn validate_page_boundary(
    request: &ResolvedPage,
    context: &PageValidation<'_>,
    mode: CursorMode,
    rows: usize,
    last_event: u64,
    cursor: Option<&Cursor>,
) -> Result<()> {
    require(
        context.scanned <= SCAN_LIMIT
            && rows <= 200
            && rows <= request.limit.get() as usize
            && rows <= context.scanned
            && request.after_seq <= request.through_seq
            && context.last_scanned >= request.after_seq
            && context.last_scanned <= request.through_seq
            && context.last_scanned.get() >= last_event
            && ((context.scanned == 0 && context.last_scanned == request.after_seq)
                || (context.scanned > 0 && context.last_scanned > request.after_seq)),
    )?;
    if let Some(cursor) = cursor {
        require(context.scanned > 0)?;
        let payload = decode_history_cursor_v1(
            context.key,
            cursor.as_str(),
            context.view,
            context.revision,
            mode,
        )?;
        require(
            payload.after_seq == context.last_scanned
                && payload.through_seq == request.through_seq
                && payload.kind == request.kind,
        )?;
    }
    Ok(())
}

impl HistoryListResult {
    /// Validate scan boundary supplied by the store, including sparse empty pages.
    pub fn validate(&self, request: &ResolvedPage, context: &PageValidation<'_>) -> Result<()> {
        require(
            self.schema == 1
                && self.through_seq == request.through_seq
                && self.cursor.is_some() == self.next_after_seq.is_some(),
        )?;
        let mut prior = request.after_seq.get();
        for event in &self.events {
            validate_event_v1(event)?;
            require(
                event.local_seq.get() > prior
                    && event.local_seq.get() <= self.through_seq.get()
                    && request.kind.is_none_or(|kind| event.kind == kind),
            )?;
            prior = event.local_seq.get();
        }
        validate_page_boundary(
            request,
            context,
            CursorMode::List,
            self.events.len(),
            prior,
            self.cursor.as_ref(),
        )?;
        if let Some(next) = self.next_after_seq {
            require(next == context.last_scanned)?;
        }
        bounded_encode(self)?;
        Ok(())
    }
    pub fn decode(
        bytes: &[u8],
        request: &ResolvedPage,
        context: &PageValidation<'_>,
    ) -> Result<Self> {
        let result: Self = decode_closed(bytes, RESPONSE_BYTES)?;
        result.validate(request, context)?;
        Ok(result)
    }
    pub fn encode(&self, request: &ResolvedPage, context: &PageValidation<'_>) -> Result<Vec<u8>> {
        self.validate(request, context)?;
        bounded_encode(self)
    }
}
closed_enum!(ExportWarning { PlaintextPersonalMetadata => "plaintext_personal_metadata" });
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryExportResult {
    pub schema: u8,
    pub jsonl: String,
    pub through_seq: SafeUInt,
    #[serde(deserialize_with = "nullable")]
    pub cursor: Option<Cursor>,
    pub warning_code: ExportWarning,
}
impl HistoryExportResult {
    pub fn validate(&self, request: &ResolvedPage, context: &PageValidation<'_>) -> Result<()> {
        require(
            self.schema == 1
                && self.through_seq == request.through_seq
                && self.jsonl.len() <= RESPONSE_BYTES
                && (self.jsonl.is_empty() || self.jsonl.ends_with('\n')),
        )?;
        let mut count = 0;
        let mut prior = request.after_seq.get();
        for line in self.jsonl.split_terminator('\n') {
            count += 1;
            require(count <= request.limit.get())?;
            let event = decode_event_v1(line.as_bytes())?;
            require(
                encode_event_v1(&event)? == line.as_bytes()
                    && event.local_seq.get() > prior
                    && event.local_seq.get() <= self.through_seq.get()
                    && request.kind.is_none_or(|kind| event.kind == kind),
            )?;
            prior = event.local_seq.get();
        }
        validate_page_boundary(
            request,
            context,
            CursorMode::Export,
            count as usize,
            prior,
            self.cursor.as_ref(),
        )?;
        bounded_encode(self)?;
        Ok(())
    }
    pub fn decode(
        bytes: &[u8],
        request: &ResolvedPage,
        context: &PageValidation<'_>,
    ) -> Result<Self> {
        let result: Self = decode_closed(bytes, RESPONSE_BYTES)?;
        result.validate(request, context)?;
        Ok(result)
    }
    pub fn encode(&self, request: &ResolvedPage, context: &PageValidation<'_>) -> Result<Vec<u8>> {
        self.validate(request, context)?;
        bounded_encode(self)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRetentionGetResult {
    pub days: Days,
    pub max_rows: u64,
    pub max_bytes: u64,
    pub revision: RetentionRevision,
    pub coverage: Coverage,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRetentionSetResult {
    pub revision: RetentionRevision,
    pub prune_pending: bool,
}
pub enum HistoryRetentionResult {
    Get(HistoryRetentionGetResult),
    Set(HistoryRetentionSetResult),
}
impl HistoryRetentionGetResult {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let result: Self = decode_closed(bytes, RESPONSE_BYTES)?;
        require(result.max_rows == 100000 && result.max_bytes == 134217728)?;
        Ok(result)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        require(self.max_rows == 100000 && self.max_bytes == 134217728)?;
        bounded_encode(self)
    }
}
impl HistoryRetentionSetResult {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        decode_closed(bytes, RESPONSE_BYTES)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        bounded_encode(self)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryChanged {
    pub latest_seq: SafeUInt,
    pub retention_revision: RetentionRevision,
    pub unavailable: bool,
}
impl HistoryChanged {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        decode_closed(bytes, RESPONSE_BYTES)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        bounded_encode(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history164_pure_boundaries() {
        let key = [0x0b; 32];
        // RFC 4231 case 2: zero-padding a short key preserves its HMAC block.
        let mut vector_key = [0u8; 32];
        vector_key[..4].copy_from_slice(b"Jefe");
        assert_eq!(
            hex(&mac(&vector_key, b"what do ya want for nothing?")).as_str(),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        let epoch = "00000000-0000-4000-8000-000000000001";
        let view = EvidenceReadViewInput {
            account_id: "synthetic-account",
            namespace: EvidenceNamespaceInput::LocalTaskEpoch(epoch),
            incarnation: SafeUInt(2),
        };
        let imported = EvidenceReadViewInput {
            namespace: EvidenceNamespaceInput::ImportedAccount("synthetic-account"),
            ..view
        };
        let bot = history_ref_v1(
            &key,
            view,
            ReferenceCategory::Bot,
            "private-structural-canary",
        )
        .unwrap();
        assert_ne!(
            bot,
            history_ref_v1(
                &key,
                view,
                ReferenceCategory::Chat,
                "private-structural-canary"
            )
            .unwrap()
        );
        assert_ne!(
            bot,
            history_ref_v1(
                &key,
                imported,
                ReferenceCategory::Bot,
                "private-structural-canary"
            )
            .unwrap()
        );
        assert!(history_ref_v1(&key, view, ReferenceCategory::Bot, &"é".repeat(129)).is_err());
        let device = PublicDeviceKey(URL_SAFE_NO_PAD.encode([1; 32]));
        let context = EventContext {
            event_id: EventId(epoch.into()),
            local_seq: PositiveSafeUInt(1),
            recorded_at_ms: SafeUInt(9),
            occurred_at_ms: None,
            observer_device_id: device.clone(),
            actor: Actor {
                kind: ActorKind::Device,
                device_id: Some(device.clone()),
            },
            action_class: ActionClass::Plugin,
            refs: References {
                bot: Some(bot.clone()),
                chat: Some(bot.clone()),
                decision: Some(bot.clone()),
                rule: None,
                policy: None,
            },
        };
        let allow = project_event_v1(
            context.clone(),
            ProducerPayload::HumanChoice(HumanChoice::Allow),
        )
        .unwrap();
        let bytes = encode_event_v1(&allow).unwrap();
        assert_eq!(decode_event_v1(&bytes).unwrap(), allow);
        assert!(!String::from_utf8(bytes.clone())
            .unwrap()
            .contains("private-structural-canary"));
        assert!(project_event_v1(
            context.clone(),
            ProducerPayload::HumanChoice(HumanChoice::Always)
        )
        .is_err());
        let mut always = context.clone();
        always.refs.rule = Some(bot.clone());
        assert_eq!(
            project_event_v1(always, ProducerPayload::HumanChoice(HumanChoice::Always))
                .unwrap()
                .summary_code,
            SummaryCode::Decision(ActionClass::Plugin, DecisionSummary::Always)
        );
        let mut invalid = allow.clone();
        invalid.policy_change = Some(PolicyChange {
            paused: Some(true),
            shell: None,
            write: None,
            plugin_change: None,
        });
        assert!(validate_event_v1(&invalid).is_err());
        invalid = allow.clone();
        invalid.reason_code = Reason::PolicyEdit;
        assert!(validate_event_v1(&invalid).is_err());
        for (tag, actor) in [
            (ProducerPayload::SavedRulePermit, ActorKind::Automatic),
            (ProducerPayload::BuiltinReadOnlyPermit, ActorKind::System),
            (
                ProducerPayload::AutomaticVerdict(Verdict::Deny),
                ActorKind::Automatic,
            ),
            (
                ProducerPayload::ReviewObservation(ReviewState::Ask),
                ActorKind::Automatic,
            ),
            (
                ProducerPayload::ReviewObservation(ReviewState::Unavailable),
                ActorKind::Automatic,
            ),
            (
                ProducerPayload::ClosedWait(WaitCause::Timeout),
                ActorKind::System,
            ),
            (
                ProducerPayload::ClosedWait(WaitCause::Cancelled),
                ActorKind::System,
            ),
            (
                ProducerPayload::ClosedWait(WaitCause::NewMessage),
                ActorKind::System,
            ),
            (
                ProducerPayload::OrphanClosure(OrphanChoice::Dismissed),
                ActorKind::System,
            ),
            (
                ProducerPayload::OrphanClosure(OrphanChoice::Cancelled),
                ActorKind::System,
            ),
        ] {
            let mut c = context.clone();
            c.actor = Actor {
                kind: actor,
                device_id: None,
            };
            if matches!(tag, ProducerPayload::SavedRulePermit) {
                c.refs.rule = Some(bot.clone());
            }
            let event = project_event_v1(c, tag).unwrap();
            assert_eq!(
                decode_event_v1(&encode_event_v1(&event).unwrap()).unwrap(),
                event
            );
        }
        let mut policy = context.clone();
        policy.action_class = ActionClass::AccountPolicy;
        policy.refs = References {
            bot: None,
            chat: None,
            decision: None,
            rule: None,
            policy: Some(PolicyRef {
                counter: SafeUInt(3),
                device_id: device,
            }),
        };
        let pause = project_event_v1(policy.clone(), ProducerPayload::LocalPause(true)).unwrap();
        let mut wrong = pause.clone();
        wrong.reason_code = Reason::UserChoice;
        wrong.policy_change = None;
        assert!(validate_event_v1(&wrong).is_err());
        policy.actor = Actor {
            kind: ActorKind::Unknown,
            device_id: None,
        };
        assert_eq!(
            project_event_v1(policy.clone(), ProducerPayload::ObservedPause(false))
                .unwrap()
                .actor
                .kind,
            ActorKind::Unknown
        );
        policy.action_class = ActionClass::BotCapability;
        policy.refs.bot = Some(bot.clone());
        let noop = PolicyChange {
            paused: None,
            shell: None,
            write: None,
            plugin_change: Some(PluginChange::Unchanged),
        };
        assert!(
            project_event_v1(policy.clone(), ProducerPayload::ObservedCapabilities(noop)).is_err()
        );
        let delta = PolicyChange {
            paused: None,
            shell: Some(false),
            write: None,
            plugin_change: Some(PluginChange::Mixed),
        };
        assert_eq!(
            project_event_v1(policy, ProducerPayload::ObservedCapabilities(delta))
                .unwrap()
                .summary_code,
            SummaryCode::PolicyChanged
        );
        let mut rule = context.clone();
        rule.action_class = ActionClass::ReviewRule;
        rule.refs = References {
            bot: None,
            chat: None,
            decision: None,
            rule: Some(bot),
            policy: None,
        };
        for (operation, summary) in [
            (RuleOperation::Added, SummaryCode::RuleAdded),
            (RuleOperation::Removed, SummaryCode::RuleRemoved),
            (RuleOperation::Replaced, SummaryCode::RuleReplaced),
        ] {
            assert_eq!(
                project_event_v1(rule.clone(), ProducerPayload::RuleEdit(operation))
                    .unwrap()
                    .summary_code,
                summary
            );
        }
        rule.refs.rule = None;
        for (operation, summary) in [
            (SettingOperation::Enabled, SummaryCode::RuleEnabled),
            (SettingOperation::Disabled, SummaryCode::RuleDisabled),
        ] {
            assert_eq!(
                project_event_v1(rule.clone(), ProducerPayload::ReviewSetting(operation))
                    .unwrap()
                    .summary_code,
                summary
            );
        }
        rule.actor = Actor {
            kind: ActorKind::System,
            device_id: None,
        };
        rule.action_class = ActionClass::Unknown;
        assert_eq!(
            project_event_v1(rule, ProducerPayload::CoverageGap)
                .unwrap()
                .summary_code,
            SummaryCode::CoverageUnavailable
        );
        let text = String::from_utf8(bytes).unwrap();
        assert!(decode_event_v1(text.replace("\"occurred_at_ms\":null,", "").as_bytes()).is_err());
        assert!(decode_event_v1(
            text.replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1)
                .as_bytes()
        )
        .is_err());
        assert!(decode_event_v1(
            text.replacen("\"schema\":1", "\"schema\":1,\"raw_body\":\"secret\"", 1)
                .as_bytes()
        )
        .is_err());
        for number in ["-0", "1.0", "9007199254740992", "\"1\""] {
            assert!(serde_json::from_str::<SafeUInt>(number).is_err());
        }
        assert!(serde_json::from_str::<PositiveSafeUInt>("0").is_err());
        assert!(
            HistoryListRequest::decode(br#"{"cursor":"h164c1.x.y","kind":"decision"}"#).is_err()
        );
        assert!(HistoryListRequest::decode(br#"{"kind":null}"#).is_err());
        assert!(HistoryListRequest::decode(br#"{"limit":1,"limit":2}"#).is_err());
        assert!(HistoryExportRequest::decode(br#"{"confirm":false}"#).is_err());
        assert!(HistoryRetentionRequest::decode_set(
            br#"{"days":30,"confirm":true,"expected_revision":0}"#
        )
        .is_err());
        let revision = PositiveSafeUInt(1);
        let cursor = HistoryCursorV1 {
            v: 1,
            namespace: history_namespace_v1(&key, view).unwrap(),
            incarnation: view.incarnation,
            retention_revision: revision,
            through_seq: SafeUInt(8),
            after_seq: SafeUInt(3),
            kind: Some(Kind::Decision),
            mode: CursorMode::List,
        };
        let encoded =
            encode_history_cursor_v1(&key, &cursor, view, revision, CursorMode::List).unwrap();
        assert_eq!(
            decode_history_cursor_v1(&key, &encoded, view, revision, CursorMode::List).unwrap(),
            cursor
        );
        assert_eq!(
            decode_history_cursor_v1(&key, &encoded, imported, revision, CursorMode::List),
            Err(HistoryError::CursorExpired)
        );
        assert_eq!(
            decode_history_cursor_v1(&key, &encoded, view, revision, CursorMode::Export),
            Err(HistoryError::CursorExpired)
        );
        assert!(
            decode_history_cursor_v1(&[3; 32], &encoded, view, revision, CursorMode::List).is_err()
        );
        assert!(decode_history_cursor_v1(
            &key,
            &(encoded.clone() + "="),
            view,
            revision,
            CursorMode::List
        )
        .is_err());
        assert!(decode_history_cursor_v1(
            &key,
            &"x".repeat(2049),
            view,
            revision,
            CursorMode::List
        )
        .is_err());
        let payload = String::from_utf8(cursor_payload(&cursor).unwrap()).unwrap();
        for bad in [
            payload.replacen("{", "{ ", 1),
            payload.replacen("\"v\":1", "\"v\":1,\"v\":1", 1),
            payload.replacen("\"v\":1", "\"v\":1,\"extra\":0", 1),
        ] {
            let authenticated_bad = format!(
                "h164c1.{}.{}",
                URL_SAFE_NO_PAD.encode(bad.as_bytes()),
                URL_SAFE_NO_PAD.encode(cursor_mac(&key, bad.as_bytes()))
            );
            assert!(decode_history_cursor_v1(
                &key,
                &authenticated_bad,
                view,
                revision,
                CursorMode::List
            )
            .is_err());
        }
        let request = ResolvedPage {
            after_seq: SafeUInt(0),
            through_seq: SafeUInt(8),
            limit: PageLimit(50),
            kind: Some(Kind::PolicyChange),
        };
        let context = PageValidation {
            key: &key,
            view,
            revision,
            scanned: 3,
            last_scanned: SafeUInt(3),
        };
        let page_cursor = HistoryCursorV1 {
            kind: request.kind,
            ..cursor.clone()
        };
        let matching =
            encode_history_cursor_v1(&key, &page_cursor, view, revision, CursorMode::List).unwrap();
        let sparse = HistoryListResult {
            schema: 1,
            events: vec![],
            through_seq: SafeUInt(8),
            next_after_seq: Some(SafeUInt(3)),
            cursor: Some(Cursor(matching)),
            coverage: Coverage {
                since_ms: SafeUInt(0),
                local_only: Confirmed,
                unavailable: false,
            },
            retention: Retention {
                days: Days::Thirty,
                oldest_available_seq: None,
            },
        };
        assert_eq!(
            HistoryListResult::decode(
                &sparse.encode(&request, &context).unwrap(),
                &request,
                &context
            )
            .unwrap(),
            sparse
        );
        let mut wrong = sparse.clone();
        wrong.cursor = Some(Cursor(encoded));
        assert!(wrong.encode(&request, &context).is_err());
        assert!(HistoryListResult::decode(
            &serde_json::to_vec(&wrong).unwrap(),
            &request,
            &context
        )
        .is_err());
        wrong = sparse.clone();
        wrong.next_after_seq = Some(SafeUInt(4));
        assert!(wrong.validate(&request, &context).is_err());
        wrong = sparse.clone();
        wrong.cursor = Some(Cursor("h164c1.x.y".into()));
        assert!(wrong.validate(&request, &context).is_err());
        for bad in [
            HistoryCursorV1 {
                after_seq: SafeUInt(4),
                ..page_cursor.clone()
            },
            HistoryCursorV1 {
                through_seq: SafeUInt(9),
                ..page_cursor.clone()
            },
            HistoryCursorV1 {
                mode: CursorMode::Export,
                ..page_cursor.clone()
            },
        ] {
            wrong = sparse.clone();
            wrong.cursor = Some(Cursor(
                encode_history_cursor_v1(&key, &bad, view, revision, bad.mode).unwrap(),
            ));
            assert!(wrong.validate(&request, &context).is_err());
        }
        assert!(sparse
            .validate(
                &request,
                &PageValidation {
                    view: imported,
                    ..context
                }
            )
            .is_err());
        assert!(sparse
            .validate(
                &request,
                &PageValidation {
                    revision: PositiveSafeUInt(2),
                    ..context
                }
            )
            .is_err());
        assert!(sparse
            .validate(
                &request,
                &PageValidation {
                    view: EvidenceReadViewInput {
                        incarnation: SafeUInt(3),
                        ..view
                    },
                    ..context
                }
            )
            .is_err());
        assert!(sparse
            .validate(
                &request,
                &PageValidation {
                    key: &[9; 32],
                    ..context
                }
            )
            .is_err());
        assert!(sparse
            .validate(
                &request,
                &PageValidation {
                    scanned: 202,
                    ..context
                }
            )
            .is_err());
        let mut stuck = sparse.clone();
        stuck.next_after_seq = Some(SafeUInt(0));
        assert!(stuck.validate(&request, &context).is_err());
        let export_request = ResolvedPage {
            kind: None,
            ..request
        };
        let export = HistoryExportResult {
            schema: 1,
            jsonl: text + "\n",
            through_seq: SafeUInt(8),
            cursor: None,
            warning_code: ExportWarning::PlaintextPersonalMetadata,
        };
        let export_context = PageValidation {
            scanned: 1,
            last_scanned: SafeUInt(1),
            ..context
        };
        assert_eq!(
            HistoryExportResult::decode(
                &export.encode(&export_request, &export_context).unwrap(),
                &export_request,
                &export_context
            )
            .unwrap(),
            export
        );
        let export_cursor = HistoryCursorV1 {
            mode: CursorMode::Export,
            ..page_cursor.clone()
        };
        let empty_export = HistoryExportResult {
            jsonl: String::new(),
            cursor: Some(Cursor(
                encode_history_cursor_v1(&key, &export_cursor, view, revision, CursorMode::Export)
                    .unwrap(),
            )),
            ..export.clone()
        };
        assert_eq!(
            HistoryExportResult::decode(
                &empty_export.encode(&request, &context).unwrap(),
                &request,
                &context
            )
            .unwrap(),
            empty_export
        );
        for bad in [
            HistoryCursorV1 {
                after_seq: SafeUInt(0),
                ..export_cursor.clone()
            },
            HistoryCursorV1 {
                after_seq: SafeUInt(4),
                ..export_cursor.clone()
            },
            HistoryCursorV1 {
                mode: CursorMode::List,
                ..export_cursor.clone()
            },
            HistoryCursorV1 {
                kind: Some(Kind::Decision),
                ..export_cursor.clone()
            },
            HistoryCursorV1 {
                through_seq: SafeUInt(9),
                ..export_cursor.clone()
            },
        ] {
            let mut wrong = empty_export.clone();
            wrong.cursor = Some(Cursor(
                encode_history_cursor_v1(&key, &bad, view, revision, bad.mode).unwrap(),
            ));
            assert!(wrong.encode(&request, &context).is_err());
            assert!(HistoryExportResult::decode(
                &serde_json::to_vec(&wrong).unwrap(),
                &request,
                &context
            )
            .is_err());
        }
        let mut malformed = empty_export.clone();
        malformed.cursor = Some(Cursor("h164c1.x.y".into()));
        assert!(malformed.validate(&request, &context).is_err());
        assert!(empty_export
            .validate(
                &request,
                &PageValidation {
                    scanned: 202,
                    ..context
                }
            )
            .is_err());
        assert!(empty_export
            .validate(
                &request,
                &PageValidation {
                    scanned: 0,
                    last_scanned: request.after_seq,
                    ..context
                }
            )
            .is_err());
        assert!(empty_export
            .validate(
                &request,
                &PageValidation {
                    view: imported,
                    ..context
                }
            )
            .is_err());
        assert!(empty_export
            .validate(
                &request,
                &PageValidation {
                    revision: PositiveSafeUInt(2),
                    ..context
                }
            )
            .is_err());
        assert!(empty_export
            .validate(
                &request,
                &PageValidation {
                    view: EvidenceReadViewInput {
                        incarnation: SafeUInt(3),
                        ..view
                    },
                    ..context
                }
            )
            .is_err());
        assert!(empty_export
            .validate(
                &request,
                &PageValidation {
                    key: &[9; 32],
                    ..context
                }
            )
            .is_err());
        let binding = InvocationOccurrenceInput {
            task_id: "task",
            owner_epoch: "owner",
            execution_id: "execution",
            invocation_id: "invocation",
            parent_id: None,
            ordinal: 1,
            attempt_id: "attempt",
            receipt_id: "receipt",
            revision: 2,
            digest: &[4; 32],
        };
        let occurrence = HistoryOccurrenceInput::Invocation {
            binding,
            stage: OccurrenceStage::Terminal,
        };
        assert!(history_occurrence_key_v1(&key, imported, occurrence).is_err());
        let first = history_occurrence_key_v1(&key, view, occurrence).unwrap();
        let changed = HistoryOccurrenceInput::Invocation {
            binding: InvocationOccurrenceInput {
                revision: 3,
                ..binding
            },
            stage: OccurrenceStage::Terminal,
        };
        assert_ne!(
            first,
            history_occurrence_key_v1(&key, view, changed).unwrap()
        );
        assert_ne!(
            first,
            history_occurrence_key_v1(&key, view, HistoryOccurrenceInput::Review(binding)).unwrap()
        );
    }
}
