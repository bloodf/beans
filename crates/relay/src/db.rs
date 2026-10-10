//! Storage behind one trait, with two backends. SQLite (`db/sqlite.rs`) is one file beside
//! one relay process. Postgres (`db/postgres.rs`) is shared by any number of relay processes,
//! so a deploy can start the new one before the old one stops; what a single process keeps in
//! memory (who is online, whom to signal, which keys were unpaired) goes through the database
//! there.
//!
//! Every SQL statement lives in a backend. `routes.rs` only decides what to ask for.

mod postgres;
mod sqlite;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::hub::Hub;
use crate::routes::{ApiError, ApiResult};

pub const KINDS: &[&str] = &[
    "roster",
    "policy",
    "chat",
    "job",
    "job_cancel",
    "job_result",
    "request",
    "response",
    "machine",
    "credentials",
    "memory_config",
    "key",
    "file",
];

/// Kinds sealed to one machine, which deletes what it consumed. One left behind (its Runner
/// never came back) is dropped by `Store::sweep` once it is stale.
pub const SEALED_KINDS: &[&str] = &["job", "job_cancel", "job_result", "request", "response"];

/// `'job', 'job_cancel', …` for an `IN (…)`.
fn sealed_kinds_sql() -> String {
    SEALED_KINDS.iter().map(|kind| format!("'{kind}'")).collect::<Vec<_>>().join(", ")
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Runs CPU- or disk-bound work off the async workers.
pub async fn blocking<T, F>(f: F) -> ApiResult<T>
where
    F: FnOnce() -> ApiResult<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|_| ApiError::internal("Database task failed"))?
}

// MARK: - Rows

pub struct Machine {
    pub machine_pubkey: String,
    pub identity_pubkey: String,
    pub box_pubkey: String,
    pub last_seen: i64,
    pub created_at: i64,
}

pub struct BlobRow {
    pub id: String,
    pub kind: String,
    pub recipient_machine_pubkey: Option<String>,
    pub seq: i64,
    /// Empty for a `file`: its bytes are in the file store under `store::key`.
    pub ciphertext: Vec<u8>,
    pub created_at: i64,
}

/// One slot of a group with the blobs it holds (a message's first and latest version, or
/// its removal), oldest first. `place` is the slot's lowest seq: where it sits in the log.
pub struct GroupSlot {
    pub place: i64,
    pub blobs: Vec<BlobRow>,
}

/// Of `(slot, place, bytes)` newest first and one longer than `limit` when there are more:
/// the slots of a page, which ends at `limit` or before the slot that would take it past
/// `max_bytes` and always holds one, and whether older slots remain.
fn page_of_slots(mut slots: Vec<(String, i64, i64)>, limit: usize, max_bytes: i64) -> (Vec<(String, i64)>, bool) {
    let mut has_more = slots.len() > limit;
    slots.truncate(limit);
    let mut bytes = 0;
    let fits = slots
        .iter()
        .take_while(|(_, _, size)| {
            bytes += size;
            bytes <= max_bytes
        })
        .count()
        .max(1)
        .min(slots.len());
    has_more |= fits < slots.len();
    slots.truncate(fits);
    (slots.into_iter().map(|(name, place, _)| (name, place)).collect(), has_more)
}

/// `chosen` (newest first) with their `rows`, turned oldest first for the Device to apply.
fn slots_with_rows(chosen: Vec<(String, i64)>, rows: Vec<(String, BlobRow)>) -> Vec<GroupSlot> {
    let mut slots: Vec<(String, GroupSlot)> = chosen.into_iter().rev().map(|(name, place)| (name, GroupSlot { place, blobs: Vec::new() })).collect();
    for (name, row) in rows {
        if let Some((_, slot)) = slots.iter_mut().find(|(slot, _)| *slot == name) {
            slot.blobs.push(row);
        }
    }
    slots.into_iter().map(|(_, slot)| slot).filter(|slot| !slot.blobs.is_empty()).collect()
}

pub struct Inserted {
    pub seq: i64,
    pub existing: bool,
}

/// A blob's bytes: in the row, or (a `file`) in the file store with only the size recorded.
pub enum Payload {
    Inline(Vec<u8>),
    InFileStore { size: i64 },
}

impl Payload {
    pub fn size(&self) -> i64 {
        match self {
            Payload::Inline(bytes) => bytes.len() as i64,
            Payload::InFileStore { size } => *size,
        }
    }
}

/// A blob's place among the versions of one thing: a message, the roster, a Device's
/// metadata. A new blob in a slot supersedes the earlier ones, so the log holds the latest
/// version instead of every version. `keep_first` spares the oldest, whose seq holds a
/// message's place in the log for a Device that replays it from the start.
pub struct Slot {
    pub name: String,
    pub keep_first: bool,
}

pub struct NewBlob {
    pub identity_pubkey: String,
    pub id: String,
    pub kind: String,
    pub recipient_machine_pubkey: Option<String>,
    pub slot: Option<Slot>,
    /// Required for roster snapshots: latest roster slot seq, or 0 if absent.
    pub expected_slot_seq: Option<i64>,
    /// What the blob belongs to, a chat to the Devices. A deleted group takes no more blobs.
    pub group: Option<String>,
    pub payload: Payload,
}

const MANAGED_RECEIPT_LIMIT: i64 = 65_536;
const MANAGED_HISTORY_BYTES: i64 = 64 * 1024 * 1024;
const MANAGED_RECEIPT_BYTES: usize = 1024;
pub(crate) const MANAGED_COMPANION_BYTES: usize = 32_768;

fn managed_capacity(count: i64, bytes: i64, additional: i64, receipt_bytes: usize) -> ApiResult<()> {
    if count >= MANAGED_RECEIPT_LIMIT || receipt_bytes > MANAGED_RECEIPT_BYTES
        || bytes.checked_add(additional).is_none_or(|total| total > MANAGED_HISTORY_BYTES)
    {
        return Err(ApiError::too_large("Managed receipt lifetime capacity exceeded"));
    }
    Ok(())
}

/// Staged encrypted companion transaction; never a plaintext managed-action DTO.
#[derive(Clone)]
pub struct ManagedCompanion {
    pub account_id: String,
    pub transaction_id: String,
    pub expected_slot_seq: i64,
    pub policy_id: String,
    pub policy_ciphertext: Vec<u8>,
    pub roster_id: String,
    pub roster_ciphertext: Vec<u8>,
    pub checkpoint_transaction_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedReceipt {
    pub schema_version: u8,
    pub account_id: String,
    pub transaction_id: String,
    pub request_sha256: String,
    pub expected_slot_seq: i64,
    pub policy_id: String,
    pub policy_ciphertext_len: i64,
    pub policy_sha256: String,
    pub policy_seq: i64,
    pub roster_id: String,
    pub roster_ciphertext_len: i64,
    pub roster_sha256: String,
    pub roster_seq: i64,
    #[serde(deserialize_with = "required_nullable")]
    pub checkpoint_transaction_id: Option<String>,
}

fn required_nullable<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

impl ManagedReceipt {
    pub fn request_commitment(&self) -> ApiResult<String> {
        use sha2::{Digest, Sha256};
        let bytes = serde_json::to_vec(&serde_json::json!([
            "beans-managed-skill-commit", 1, self.account_id, self.transaction_id, self.expected_slot_seq,
            [self.policy_id, self.policy_ciphertext_len, self.policy_sha256, "managed_skill_policy", null, null, null],
            [self.roster_id, self.roster_ciphertext_len, self.roster_sha256, "roster", "roster", null, null],
            self.checkpoint_transaction_id
        ])).map_err(|_| ApiError::internal("Request commitment serialization failed"))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

impl ManagedCompanion {
    pub fn receipt(&self, policy_seq: i64, roster_seq: i64) -> ApiResult<ManagedReceipt> {
        crate::routes::validate_managed_companion(self)?;
        use sha2::{Digest, Sha256};
        let mut receipt = ManagedReceipt {
            schema_version: 1, account_id: self.account_id.clone(), transaction_id: self.transaction_id.clone(),
            request_sha256: String::new(), expected_slot_seq: self.expected_slot_seq,
            policy_id: self.policy_id.clone(), policy_ciphertext_len: self.policy_ciphertext.len() as i64,
            policy_sha256: format!("{:x}", Sha256::digest(&self.policy_ciphertext)), policy_seq,
            roster_id: self.roster_id.clone(), roster_ciphertext_len: self.roster_ciphertext.len() as i64,
            roster_sha256: format!("{:x}", Sha256::digest(&self.roster_ciphertext)), roster_seq,
            checkpoint_transaction_id: self.checkpoint_transaction_id.clone(),
        };
        receipt.request_sha256 = receipt.request_commitment()?;
        Ok(receipt)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedCut {
    pub schema_version: u8,
    pub context: String,
    pub cut_token: String,
    pub through: i64,
    pub next_since: i64,
    pub done: bool,
    pub blobs: Vec<ManagedCutBlob>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "subtype", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagedCutBlob {
    Ordinary { id: String, seq: i64, ciphertext: Vec<u8> },
    Managed { receipt: ManagedReceipt, policy_ciphertext: Vec<u8>, roster_ciphertext: Vec<u8> },
}

impl ManagedCutBlob {
    fn seq(&self) -> i64 {
        match self { Self::Ordinary { seq, .. } => *seq, Self::Managed { receipt, .. } => receipt.policy_seq }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayClaims {
    account: String, incarnation: String, context: String, through: i64, expires: i64,
    cursor: i64, pages: u32, envelopes: i64, bytes: i64,
}

struct ReplaySession { nonce: String, key: [u8; 32] }

impl Default for ReplaySession {
    fn default() -> Self {
        use rand::RngCore;
        let mut key = [0; 32];
        rand::thread_rng().fill_bytes(&mut key);
        Self { nonce: uuid::Uuid::new_v4().to_string(), key }
    }
}

impl ReplaySession {
    fn claims(&self, account: &str, incarnation: &str, context: &str, token: &str, through: i64, since: i64, head: i64) -> ApiResult<ReplayClaims> {
        use hmac::{Hmac, Mac};
        if token.is_empty() {
            if since != 0 || through != 0 || !context.is_empty() { return Err(ApiError::bad_request("First replay requires since=0 and no cut")); }
            return Ok(ReplayClaims { account: account.into(), incarnation: incarnation.into(), context: format!("{incarnation}:{}", self.nonce), through: head, expires: now() + 300, cursor: 0, pages: 0, envelopes: 0, bytes: 0 });
        }
        if token.len() > 4096 { return Err(ApiError::bad_request("Invalid replay token")); }
        let (payload, signature) = token.split_once('.').ok_or_else(|| ApiError::bad_request("Invalid replay token"))?;
        let bytes = crate::auth::b64url_decode(payload)?;
        let signature = crate::auth::b64url_decode(signature)?;
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&self.key).map_err(|_| ApiError::internal("Replay key unavailable"))?;
        mac.update(&bytes);
        mac.verify_slice(&signature).map_err(|_| ApiError::conflict("Replay token authentication failed"))?;
        let claims: ReplayClaims = serde_json::from_slice(&bytes).map_err(|_| ApiError::bad_request("Invalid replay claims"))?;
        if claims.account != account || claims.incarnation != incarnation || claims.context != format!("{incarnation}:{}", self.nonce)
            || claims.context != context || claims.through != through || claims.cursor != since || claims.expires <= now()
            || claims.pages >= 65_537 || since < 0 || since > through || through > head
        { return Err(ApiError::conflict("Replay context, cut or cursor changed")); }
        Ok(claims)
    }

    fn page(&self, mut claims: ReplayClaims, rows: Vec<ManagedCutBlob>, has_more: bool) -> ApiResult<ManagedCut> {
        use hmac::{Hmac, Mac};
        if has_more && rows.is_empty() { return Err(ApiError::conflict("Empty nonfinal replay page")); }
        let mut blobs = Vec::new();
        let mut page_bytes = 0;
        let mut last = claims.cursor;
        let done = !has_more;
        for row in rows {
            let seq = row.seq();
            if seq <= last || seq > claims.through { return Err(ApiError::conflict("Invalid replay order")); }
            if let ManagedCutBlob::Managed { receipt, policy_ciphertext, roster_ciphertext } = &row {
                use sha2::Digest;
                if receipt.schema_version != 1 || receipt.account_id != claims.account || receipt.policy_seq <= 0
                    || receipt.policy_seq >= receipt.roster_seq || receipt.roster_seq > claims.through
                    || receipt.request_sha256 != receipt.request_commitment()?
                    || receipt.policy_ciphertext_len != policy_ciphertext.len() as i64
                    || receipt.roster_ciphertext_len != roster_ciphertext.len() as i64
                    || receipt.policy_sha256 != format!("{:x}", sha2::Sha256::digest(policy_ciphertext))
                    || receipt.roster_sha256 != format!("{:x}", sha2::Sha256::digest(roster_ciphertext))
                { return Err(ApiError::conflict("Replay companion linkage invalid or cut bisected")); }
            }
            let transport_bound = match &row {
                ManagedCutBlob::Ordinary { ciphertext, .. } => ciphertext.len().saturating_mul(4).saturating_add(8192),
                ManagedCutBlob::Managed { policy_ciphertext, roster_ciphertext, .. } => policy_ciphertext.len().saturating_add(roster_ciphertext.len()).saturating_mul(4).saturating_add(8192),
            };
            if transport_bound > 8 * 1024 * 1024 { return Err(ApiError::too_large("Replay envelope exceeds encoded page bound")); }
            let size = serde_json::to_vec(&row).map_err(|_| ApiError::internal("Replay serialization failed"))?.len();
            if !blobs.is_empty() || page_bytes + size + 8192 > 8 * 1024 * 1024 {
                return Err(ApiError::too_large("Replay envelope exceeds page bound"));
            }
            page_bytes += size;
            claims.envelopes += 1;
            claims.bytes += match &row { ManagedCutBlob::Ordinary { ciphertext, .. } => ciphertext.len() as i64, ManagedCutBlob::Managed { policy_ciphertext, .. } => policy_ciphertext.len() as i64 };
            if claims.envelopes > 1_048_576 || claims.bytes > 1024 * 1024 * 1024 { return Err(ApiError::too_large("Replay history bound exceeded")); }
            last = match &row { ManagedCutBlob::Ordinary { seq, .. } => *seq, ManagedCutBlob::Managed { receipt, .. } => receipt.roster_seq };
            blobs.push(row);
        }
        claims.pages += 1;
        claims.cursor = if done { claims.through } else { last };
        let payload = serde_json::to_vec(&claims).map_err(|_| ApiError::internal("Replay token serialization failed"))?;
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&self.key).map_err(|_| ApiError::internal("Replay key unavailable"))?;
        mac.update(&payload);
        let cut_token = format!("{}.{}", crate::auth::b64url_encode(&payload), crate::auth::b64url_encode(&mac.finalize().into_bytes()));
        Ok(ManagedCut { schema_version: 1, context: claims.context, cut_token, through: claims.through, next_since: claims.cursor, done, blobs })
    }
}

fn validate_replay_payload_size(policy_bytes: i64, roster_bytes: i64) -> ApiResult<()> {
    if policy_bytes < 0 || roster_bytes < 0
        || policy_bytes.checked_add(roster_bytes).and_then(|bytes| bytes.checked_mul(4)).and_then(|bytes| bytes.checked_add(8192)).is_none_or(|bytes| bytes > 8 * 1024 * 1024)
    {
        return Err(ApiError::too_large("Replay envelope exceeds encoded page bound"));
    }
    Ok(())
}

fn validate_managed_cut(limit: i64) -> ApiResult<()> {
    if !(1..=1024).contains(&limit) { return Err(ApiError::bad_request("Replay page limit out of range")); }
    Ok(())
}

/// What a deleted identity leaves for the caller to finish: its machines' tokens and sockets
/// to end, and its `file` objects to remove.
pub struct DeletedIdentity {
    pub machines: Vec<String>,
    pub files: Vec<String>,
}

/// Totals for `/metrics`. The relay reads no content, so these are all it knows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    pub identities: i64,
    pub machines: i64,
    /// Machines seen in the last day, seven days, and thirty days.
    pub active_machines: [i64; 3],
    pub revoked_machines: i64,
    pub deleted_groups: i64,
    /// `(kind, count, bytes)`.
    pub blobs: Vec<(String, i64, i64)>,
    pub usage_bytes: i64,
    pub largest_identity_bytes: i64,
    /// `(platform, count)`.
    pub push_tokens: Vec<(String, i64)>,
}

/// Where a phone takes pushes: its APNs or FCM device token, one per machine.
#[derive(Debug, Clone)]
pub struct PushToken {
    pub machine_pubkey: String,
    pub platform: String,
    pub token: String,
    /// `sandbox` or `production`; APNs keeps a host for each.
    pub environment: String,
}

// MARK: - What one process keeps in memory

/// The keys of unpaired machines, so a bearer token issued before the unpairing dies with
/// it. Loaded from the database at startup and kept current by `Event::Revoked`.
#[derive(Default)]
pub struct Revoked {
    keys: Mutex<HashSet<String>>,
}

impl Revoked {
    pub fn contains(&self, machine_pubkey: &str) -> bool {
        self.lock().contains(machine_pubkey)
    }

    pub fn insert(&self, machine_pubkey: &str) {
        self.lock().insert(machine_pubkey.to_string());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.keys.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// This process's sockets and revoked keys: where an `Event` lands.
#[derive(Default)]
pub struct Local {
    pub hub: Hub,
    pub revoked: Revoked,
    replay: ReplaySession,
}

/// Something every relay process has to hear about. With SQLite there is one process and the
/// event is delivered in place; with Postgres it goes through `NOTIFY` to all of them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Event {
    /// The identity has a new blob. One sealed to a machine concerns that machine alone.
    Blobs { identity: String, recipient: Option<String> },
    /// The identity's machine list or a machine's presence changed.
    Machines { identity: String },
    /// A machine was unpaired: its tokens die and its sockets close.
    Revoked { identity: String, machine: String },
}

impl Local {
    pub fn deliver(&self, event: &Event) {
        match event {
            Event::Blobs { identity, recipient } => self.hub.blobs(identity, recipient.as_deref()),
            Event::Machines { identity } => self.hub.machines(identity),
            Event::Revoked { identity, machine } => {
                self.revoked.insert(machine);
                self.hub.kick(identity, machine);
                self.hub.machines(identity);
            }
        }
    }
}

// MARK: - The backend

#[async_trait]
pub trait Store: Send + Sync {
    fn describe(&self) -> String;

    // Identities and machines

    /// Registers the identity (idempotent) and attests one machine. `410` for a revoked key,
    /// `409` for an identity known under another content key.
    async fn register_identity(&self, identity_pubkey: &str, content_pubkey: &str, machine_pubkey: &str, box_pubkey: &str, attestation: &str) -> ApiResult<()>;
    /// A paired machine (`by`) attests another for its identity. `404` when `by` is not one of
    /// the identity's machines (`410` when it was unpaired), `410` for a revoked key, `409` for
    /// a key already paired with other keys or to another identity. The same keys again change
    /// nothing.
    async fn attest_machine(&self, identity_pubkey: &str, by: &str, machine_pubkey: &str, box_pubkey: &str, attestation: &str) -> ApiResult<()>;
    /// `404` for an unknown machine, `410` for a revoked one.
    async fn create_challenge(&self, nonce: &str, machine_pubkey: &str, expires_at: i64) -> ApiResult<()>;
    /// Spends the challenge (a nonce is good once), checks it was this machine's and is still
    /// good, writes `last_seen`, and answers with the machine.
    async fn redeem_challenge(&self, nonce: &str, machine_pubkey: &str) -> ApiResult<Machine>;
    async fn machines_for(&self, identity_pubkey: &str) -> ApiResult<Vec<Machine>>;
    async fn touch_machine(&self, machine_pubkey: &str) -> ApiResult<()>;
    /// Unpairs a machine of this identity: its row goes, its key is remembered as revoked, and
    /// the envelopes sealed to it go with it. False when the identity has no such machine.
    async fn revoke_machine(&self, identity_pubkey: &str, machine_pubkey: &str) -> ApiResult<bool>;
    async fn revoked_machines(&self) -> ApiResult<Vec<String>>;
    /// Deletes the identity and everything the relay holds for it. With `revoke` its machines'
    /// keys are remembered as revoked, so every Device gets `410` and forgets the identity;
    /// without, a Device that comes back registers again and keeps what it has.
    /// True account teardown also deletes retained managed receipts and encrypted history,
    /// regardless of `revoke`; blob/group deletion and roster supersession never do.
    async fn delete_identity(&self, identity_pubkey: &str, revoke: bool) -> ApiResult<DeletedIdentity>;
    /// Identities with no sign of life since `before`: registered earlier, every machine last
    /// seen earlier, the newest blob older, and no socket open now.
    async fn inactive_identities(&self, before: i64) -> ApiResult<Vec<String>>;
    /// Sets `usage` to what the blobs add up to wherever the two disagree; returns how many
    /// identities that was.
    async fn recount_usage(&self) -> ApiResult<u64>;

    // Blobs

    /// What `insert_blob` would say before a `file`'s object goes up: the seq of a known id,
    /// or a refusal (deleted group, quota). `None` means go ahead; the insert decides for real.
    async fn precheck_blob(&self, identity_pubkey: &str, id: &str, group: Option<&str>, size: i64, quota_bytes: u64) -> ApiResult<Option<Inserted>>;
    /// Stores a blob under the identity's next sequence number. A known id returns its
    /// existing seq. `quota_bytes` of 0 means unlimited.
    async fn insert_blob(&self, blob: NewBlob, quota_bytes: u64) -> ApiResult<Inserted>;
    /// Staged only. Receipt lookup precedes CAS; exact retries return the original receipt.
    async fn commit_managed_companion(&self, request: ManagedCompanion, quota_bytes: u64) -> ApiResult<ManagedReceipt>;
    async fn managed_receipt(&self, account_id: &str, transaction_id: &str) -> ApiResult<Option<ManagedReceipt>>;
    /// Empty context/token and through=0 capture first committed snapshot; continuation is authenticated.
    async fn managed_cut(&self, account_id: &str, context: &str, cut_token: &str, through: i64, since: i64, limit: i64) -> ApiResult<ManagedCut>;
    /// Staged maintenance primitive: rotate restored account storage context before admission.
    async fn reset_managed_replay_context(&self, account_id: &str) -> ApiResult<()>;
    /// A page of blobs after `since` that this machine may see (unaddressed ones and its own
    /// envelopes), and the identity's head seq. The page ends at `limit` rows or before the
    /// row that would take it past `max_bytes`, and always holds one row when there is one.
    async fn blobs_since(&self, identity_pubkey: &str, machine_pubkey: &str, since: i64, kinds: &[String], limit: i64, max_bytes: i64) -> ApiResult<(Vec<BlobRow>, i64)>;
    /// A group's `chat` blobs a slot at a time, backwards: the `limit` slots whose place is
    /// below `before`, oldest first, and whether older ones remain. A message keeps its
    /// place however late its last version landed, so pages read backwards still give the
    /// transcript in order.
    async fn group_page(&self, identity_pubkey: &str, group: &str, before: i64, limit: usize, max_bytes: i64) -> ApiResult<(Vec<GroupSlot>, bool)>;
    async fn blob(&self, identity_pubkey: &str, machine_pubkey: &str, id: &str) -> ApiResult<Option<BlobRow>>;
    /// Atomically deletes an individually removable blob and releases its bytes. Roster and
    /// policy rows are protected: they and missing ids return `None`. No pre-delete lookup.
    /// The caller removes a deleted `file`'s object afterwards.
    async fn delete_blob(&self, identity_pubkey: &str, id: &str) -> ApiResult<Option<String>>;
    /// Deletes every blob of a group and marks the group deleted for good. Returns the ids of
    /// the `file` blobs among them; the caller removes their objects afterwards.
    async fn delete_group(&self, identity_pubkey: &str, group: &str) -> ApiResult<Vec<String>>;

    /// Of `(identity, blob id)` pairs found in the file store, those with no row although
    /// their identity is known here. An object of an identity this database never heard of is
    /// not an orphan: the file store may belong to another database.
    async fn orphans(&self, files: &[(String, String)]) -> ApiResult<Vec<(String, String)>>;

    // Push tokens

    async fn set_push_token(&self, identity_pubkey: &str, token: &PushToken) -> ApiResult<()>;
    async fn delete_push_token(&self, machine_pubkey: &str) -> ApiResult<()>;
    /// The identity's tokens, leaving out the machine that asks for the push.
    async fn push_tokens_for(&self, identity_pubkey: &str, except_machine: &str) -> ApiResult<Vec<PushToken>>;

    // Pairing mailbox. A nonce that is unknown or expired is `404`; one that belongs to
    // another identity is `403`.

    async fn create_pairing(&self, nonce: &str, identity_pubkey: &str, expires_at: i64) -> ApiResult<()>;
    async fn delete_pairing(&self, nonce: &str, identity_pubkey: &str) -> ApiResult<()>;
    /// `409` when the pairing already holds a request.
    async fn post_pair_request(&self, nonce: &str, ciphertext: &[u8]) -> ApiResult<()>;
    async fn pair_request(&self, nonce: &str, identity_pubkey: &str) -> ApiResult<Option<Vec<u8>>>;
    async fn post_pair_reply(&self, nonce: &str, identity_pubkey: &str, ciphertext: &[u8]) -> ApiResult<()>;
    async fn pair_reply(&self, nonce: &str) -> ApiResult<Option<Vec<u8>>>;

    /// Housekeeping, once a minute: expired challenges and pairings go, and with Postgres
    /// this process says it is alive and clears the sockets of processes that are not.
    async fn tick(&self) -> ApiResult<()>;
    /// Housekeeping, once an hour: sealed envelopes made before `sealed_before` that nobody
    /// consumed go, with their bytes given back, and so do the marks of groups deleted before
    /// `groups_before`. Returns how many envelopes went.
    async fn sweep(&self, sealed_before: i64, groups_before: i64) -> ApiResult<u64>;

    async fn stats(&self) -> ApiResult<Stats>;

    // Presence and events. `Local` has this process's answer; a shared backend widens it to
    // all of them.

    /// A machine's sync socket opened here. True when it had none anywhere: it came online.
    async fn socket_opened(&self, identity_pubkey: &str, machine_pubkey: &str, socket_id: u64, first_here: bool) -> ApiResult<bool>;
    /// True when that was the machine's last socket anywhere: it went offline.
    async fn socket_closed(&self, identity_pubkey: &str, machine_pubkey: &str, socket_id: u64, last_here: bool) -> ApiResult<bool>;
    async fn online(&self, identity_pubkey: &str) -> ApiResult<HashSet<String>>;
    async fn publish(&self, event: Event);
    /// The process is stopping: its sockets are no longer anyone's presence.
    async fn close(&self);
}

/// `postgres://…` or `postgresql://…` opens Postgres; anything else is a SQLite path.
pub async fn open(location: &str, local: Arc<Local>) -> anyhow::Result<Arc<dyn Store>> {
    let store: Arc<dyn Store> = if location.starts_with("postgres://") || location.starts_with("postgresql://") {
        Arc::new(postgres::Postgres::open(location, local.clone()).await?)
    } else {
        Arc::new(sqlite::Sqlite::open(location, local.clone())?)
    };
    for key in store.revoked_machines().await.map_err(|e| anyhow::anyhow!("{e:?}"))? {
        local.revoked.insert(&key);
    }
    Ok(store)
}

#[cfg(test)]
mod tests;
