//! Pure artifact wire validation and canonical identity. This module performs no I/O or admission.

use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub const MAX_METADATA_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_TEXT_BYTES: usize = 65_536;
pub const MAX_TEXT_CHARS: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactFailure {
    Invalid,
    AuthorityChanged,
    RequesterRevoked,
    Removed,
    Incomplete,
    Conflict { current_revision: String, current_hash: String },
    RequestBodyChanged,
    Storage,
    WorkspaceUncertain,
    SyncBlocked,
}

impl fmt::Display for ArtifactFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "artifact_invalid",
            Self::AuthorityChanged => "artifact_authority_changed",
            Self::RequesterRevoked => "artifact_requester_revoked",
            Self::Removed => "artifact_removed",
            Self::Incomplete => "artifact_incomplete",
            Self::Conflict { .. } => "artifact_conflict",
            Self::RequestBodyChanged => "artifact_request_body_changed",
            Self::Storage => "artifact_storage",
            Self::WorkspaceUncertain => "artifact_workspace_uncertain",
            Self::SyncBlocked => "artifact_sync_blocked",
        })
    }
}
impl std::error::Error for ArtifactFailure {}

fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRevision {
    pub version: u8,
    pub id: String,
    pub revision_id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub content_hash: String,
    #[serde(deserialize_with = "nullable")]
    pub parent_revision: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub parent_content_hash: Option<String>,
    pub file_id: String,
    pub chat_id: String,
    #[serde(deserialize_with = "nullable")]
    pub bot_id: Option<String>,
    pub runner_id: String,
    #[serde(deserialize_with = "nullable")]
    pub card_id: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub attachment_id: Option<String>,
    pub created_at: f64,
    #[serde(deserialize_with = "nullable")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemoveOp { Remove }
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemoveChatOp { RemoveChat }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRemove {
    pub version: u8,
    pub op: RemoveOp,
    pub id: String,
    pub chat_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRemoveChat {
    pub version: u8,
    pub op: RemoveChatOp,
    pub chat_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(untagged)]
pub enum ArtifactEnvelope {
    Revision(ArtifactRevision),
    Remove(ArtifactRemove),
    RemoveChat(ArtifactRemoveChat),
}

// Dispatch only after rejecting duplicate keys: serde's untagged buffering would lose them.
impl<'de> Deserialize<'de> for ArtifactEnvelope {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct EnvelopeVisitor;
        impl<'de> serde::de::Visitor<'de> for EnvelopeVisitor {
            type Value = ArtifactEnvelope;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result { f.write_str("an artifact object") }
            fn visit_map<M: serde::de::MapAccess<'de>>(self, mut input: M) -> Result<Self::Value, M::Error> {
                let mut fields = serde_json::Map::new();
                while let Some(key) = input.next_key::<String>()? {
                    if fields.contains_key(&key) { return Err(serde::de::Error::custom("duplicate artifact field")); }
                    fields.insert(key, input.next_value::<serde_json::Value>()?);
                }
                let op = fields.get("op").cloned();
                let value = serde_json::Value::Object(fields);
                let envelope = match op.as_ref().and_then(serde_json::Value::as_str) {
                    Some("remove") => serde_json::from_value(value).map(ArtifactEnvelope::Remove),
                    Some("remove_chat") => serde_json::from_value(value).map(ArtifactEnvelope::RemoveChat),
                    None if op.is_none() => serde_json::from_value(value).map(ArtifactEnvelope::Revision),
                    _ => return Err(serde::de::Error::custom("invalid artifact operation")),
                }.map_err(|_| serde::de::Error::custom("invalid artifact fields"))?;
                envelope.validate().map_err(serde::de::Error::custom)?;
                Ok(envelope)
            }
        }
        d.deserialize_map(EnvelopeVisitor)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSave {
    pub id: String,
    pub expected_revision: String,
    pub expected_hash: String,
    pub content: String,
    #[serde(deserialize_with = "nullable")]
    pub path: Option<String>,
    pub request_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncState { Pending, Synced, Blocked }
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceState { Reserved, AttemptAdmitted, Published, Unindexed, Uncertain }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactResult {
    pub id: String,
    pub revision_id: String,
    pub content_hash: String,
    pub workspace: WorkspaceState,
    pub sync: SyncState,
}

/// Host-issued identity, not a wire authentication claim. No task lease is minted here.
#[derive(Clone, PartialEq, Eq)]
pub struct ArtifactAuthority {
    pub account_id: String,
    pub incarnation: u64,
    pub requester: String,
    pub runner_id: String,
    pub owner_epoch: String,
    pub account_epoch: String,
}
#[derive(Clone, PartialEq, Eq)]
pub enum ArtifactOrigin {
    Proposal { chat_id: String, bot_id: String, card_id: String },
    Pin { chat_id: String, attachment_id: String },
    UserRevision { authenticated_request_id: String },
}

fn structural(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
}
fn hash(value: &str) -> bool { value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) }
fn artifact_id(value: &str) -> bool {
    value.strip_prefix("art-").is_some_and(|hex| hex.len() == 48 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}
fn revision_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.hyphenated().to_string() == value)
}
fn relative_path(value: &str) -> bool {
    !value.is_empty() && value.chars().count() <= 240 && !value.chars().any(|c| c.is_control() || c == '\\')
        && value.split('/').all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains(':'))
}
fn optional(value: Option<&str>, valid: impl Fn(&str) -> bool) -> bool { value.is_none_or(valid) }
fn require(valid: bool) -> Result<(), ArtifactFailure> { if valid { Ok(()) } else { Err(ArtifactFailure::Invalid) } }

impl ArtifactRevision {
    pub fn validate(&self) -> Result<(), ArtifactFailure> {
        require(self.version == 1 && artifact_id(&self.id) && revision_id(&self.revision_id)
            && structural(&self.name) && structural(&self.mime) && self.mime.contains('/')
            && self.size <= MAX_FILE_BYTES && hash(&self.content_hash)
            && self.file_id.strip_suffix(".file") == Some(self.revision_id.as_str())
            && structural(&self.chat_id) && structural(&self.runner_id)
            && optional(self.bot_id.as_deref(), structural) && optional(self.card_id.as_deref(), structural)
            && optional(self.attachment_id.as_deref(), structural) && self.created_at.is_finite()
            && optional(self.path.as_deref(), relative_path))?;
        match (&self.parent_revision, &self.parent_content_hash) {
            (None, None) => Ok(()),
            (Some(parent), Some(parent_hash)) => require(revision_id(parent) && parent != &self.revision_id && hash(parent_hash)),
            _ => Err(ArtifactFailure::Invalid),
        }
    }

    /// Check a concrete immutable parent; absence is incomplete, never fresh root authority.
    pub fn validate_parent(&self, parent: Option<&Self>) -> Result<(), ArtifactFailure> {
        self.validate()?;
        match (self.parent_revision.as_deref(), parent) {
            (None, None) => Ok(()),
            (Some(_), None) => Err(ArtifactFailure::Incomplete),
            (None, Some(_)) => Err(ArtifactFailure::Invalid),
            (Some(expected), Some(parent)) => {
                parent.validate()?;
                require(expected == parent.revision_id && self.parent_content_hash.as_deref() == Some(parent.content_hash.as_str())
                    && self.id == parent.id && self.chat_id == parent.chat_id && self.runner_id == parent.runner_id)
            }
        }
    }

    pub fn verify_bytes(&self, bytes: &[u8]) -> Result<(), ArtifactFailure> {
        self.validate()?;
        require(bytes.len() as u64 == self.size && sha256_hex(bytes) == self.content_hash)
    }
}

impl ArtifactEnvelope {
    pub fn parse(bytes: &[u8]) -> Result<Self, ArtifactFailure> {
        require(bytes.len() <= MAX_METADATA_BYTES)?;
        serde_json::from_slice(bytes).map_err(|_| ArtifactFailure::Invalid)
    }
    pub fn validate(&self) -> Result<(), ArtifactFailure> {
        match self {
            Self::Revision(revision) => revision.validate(),
            Self::Remove(control) => require(control.version == 1 && artifact_id(&control.id) && structural(&control.chat_id)),
            Self::RemoveChat(control) => require(control.version == 1 && structural(&control.chat_id)),
        }
    }
}
impl ArtifactSave {
    pub fn parse(bytes: &[u8]) -> Result<Self, ArtifactFailure> {
        require(bytes.len() <= MAX_METADATA_BYTES)?;
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ArtifactFailure::Invalid)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), ArtifactFailure> {
        require(artifact_id(&self.id) && revision_id(&self.expected_revision) && hash(&self.expected_hash)
            && self.content.len() <= MAX_TEXT_BYTES && self.content.chars().count() <= MAX_TEXT_CHARS
            && optional(self.path.as_deref(), relative_path) && structural(&self.request_id))
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    use std::fmt::Write;
    for byte in digest { write!(&mut output, "{byte:02x}").expect("writing to String"); }
    output
}
fn identity(domain: &[u8], parts: &[&str]) -> Result<String, ArtifactFailure> {
    let mut digest = Sha256::new();
    digest.update(domain);
    digest.update([0]);
    for part in parts {
        require(structural(part))?;
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    let digest = digest.finalize();
    let mut output = String::with_capacity(48);
    use std::fmt::Write;
    for byte in &digest[..24] { write!(&mut output, "{byte:02x}").expect("writing to String"); }
    Ok(output)
}
pub fn proposal_id(chat: &str, bot: &str, card: &str) -> Result<String, ArtifactFailure> {
    Ok(format!("art-{}", identity(b"beans.artifact.proposal.v1", &[chat, bot, card])?))
}
pub fn pin_id(chat: &str, attachment: &str) -> Result<String, ArtifactFailure> {
    Ok(format!("art-{}", identity(b"beans.artifact.pin.v1", &[chat, attachment])?))
}
pub fn removed_chat_id(chat: &str) -> Result<String, ArtifactFailure> {
    Ok(format!("ach-{}.removed", identity(b"beans.artifact.chat-removal.v1", &[chat])?))
}

/// Canonical private request commitment. Authentication and lifecycle checks belong to the caller.
/// Generated revision ids/times and transport signature/time are deliberately excluded.
pub fn acceptance_fingerprint(
    authority: &ArtifactAuthority, origin: &ArtifactOrigin, request_id: &str,
    revision: &ArtifactRevision, expected: Option<(&str, &str)>,
) -> Result<[u8; 32], ArtifactFailure> {
    revision.validate()?;
    require(structural(&authority.account_id) && structural(&authority.requester)
        && structural(&authority.runner_id) && structural(request_id)
        && structural(&authority.owner_epoch) && structural(&authority.account_epoch)
        && authority.runner_id == revision.runner_id)?;
    if let Some((parent, parent_hash)) = expected {
        require(revision_id(parent) && hash(parent_hash)
            && revision.parent_revision.as_deref() == Some(parent)
            && revision.parent_content_hash.as_deref() == Some(parent_hash))?;
    } else { require(revision.parent_revision.is_none())?; }
    let origin_tuple = match origin {
        ArtifactOrigin::Proposal { chat_id, bot_id, card_id } => {
            require(chat_id == &revision.chat_id && revision.bot_id.as_ref() == Some(bot_id)
                && revision.card_id.as_ref() == Some(card_id) && revision.attachment_id.is_none())?;
            if expected.is_none() { require(proposal_id(chat_id, bot_id, card_id)? == revision.id)?; }
            serde_json::json!(["proposal", chat_id, bot_id, card_id])
        }
        ArtifactOrigin::Pin { chat_id, attachment_id } => {
            require(expected.is_none() && chat_id == &revision.chat_id
                && revision.attachment_id.as_ref() == Some(attachment_id) && revision.card_id.is_none()
                && pin_id(chat_id, attachment_id)? == revision.id)?;
            serde_json::json!(["pin", chat_id, attachment_id])
        }
        ArtifactOrigin::UserRevision { authenticated_request_id } => {
            require(expected.is_some() && authenticated_request_id == request_id
                && revision.card_id.is_none() && revision.attachment_id.is_none())?;
            serde_json::json!(["user_revision", authenticated_request_id])
        }
    };
    let canonical = serde_json::to_vec(&serde_json::json!([
        "beans-artifact-accept-v1", authority.account_id, authority.runner_id,
        authority.requester, request_id, origin_tuple, revision.id,
        expected.map(|p| p.0), expected.map(|p| p.1), revision.name, revision.mime,
        revision.size, revision.content_hash, revision.chat_id, revision.bot_id,
        revision.path, revision.card_id, revision.attachment_id
    ])).map_err(|_| ArtifactFailure::Invalid)?;
    Ok(Sha256::digest(canonical).into())
}
