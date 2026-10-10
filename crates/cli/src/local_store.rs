//! The local SQLite store shared by the desktop CLI and the embedded phone core: account
//! metadata, sync bookkeeping, messages, and the relay outbox.

use std::path::Path;
use std::sync::Mutex;

use anyhow::Context;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::de::DeserializeOwned;

use crate::app::{OutboxItem, SentJob, Slot, State};
use crate::model::{Author, Body, LiveTurn, Message};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SchemaProfile { Empty, PreTask, InitialTask, Core, Scheduled, Current }

// Compare SQL tokens, not incidental whitespace or identifier quoting. The original
// DDL is the single definition of column, default, key, index and CHECK semantics.
fn schema_tokens(sql: &str) -> anyhow::Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_whitespace() || ch == ';' { continue; }
        if matches!(ch, '\'' | '"' | '`' | '[') {
            let end = if ch == '[' { ']' } else { ch };
            let mut value = String::new();
            let mut closed = false;
            while let Some(c) = chars.next() {
                if c == end {
                    if chars.peek() == Some(&end) { chars.next(); value.push(c); }
                    else { closed = true; break; }
                } else { value.push(c); }
            }
            anyhow::ensure!(closed, "Unterminated schema token");
            tokens.push(if ch == '\'' { format!("'{}'", value.replace('\'', "''")) } else { value.to_ascii_lowercase() });
        } else if ch.is_ascii_alphanumeric() || ch == '_' {
            let mut word = String::from(ch);
            while chars.peek().is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_') { word.push(chars.next().unwrap()); }
            tokens.push(word.to_ascii_lowercase());
        } else {
            anyhow::ensure!(matches!(ch, '(' | ')' | ',' | '=' | '>' | '<' | '+' | '-'), "Unsupported schema token");
            tokens.push(ch.to_string());
        }
    }
    for at in [3, 2] {
        if tokens.get(at).map(String::as_str) == Some("if") && tokens.get(at+1).map(String::as_str) == Some("not") && tokens.get(at+2).map(String::as_str) == Some("exists") { tokens.drain(at..at+3); }
    }
    Ok(tokens)
}

fn task_schema_name(name: &str) -> bool { name.starts_with("task_") || name == "local_tasks" || name.starts_with("routine_schedule_") || name == "routine_check_commits" }

fn classify_schema(conn: &Connection) -> anyhow::Result<SchemaProfile> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    classify_schema_version(conn, version)
}

fn classify_schema_version(conn: &Connection, version: i64) -> anyhow::Result<SchemaProfile> {
    let application: u32 = conn.pragma_query_value(None, "application_id", |r| r.get(0))?;
    let objects: Vec<(String, String, String, Option<String>)> = conn.prepare("SELECT name,type,tbl_name,sql FROM sqlite_master WHERE substr(name,1,7) != 'sqlite_'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
    if objects.is_empty() {
        anyhow::ensure!(version == 0 && (application == 0 || application == crate::config::SQLITE_APPLICATION_ID), "Unsupported empty schema identity");
        return Ok(SchemaProfile::Empty);
    }
    anyhow::ensure!(application == crate::config::SQLITE_APPLICATION_ID && (1..=4).contains(&version), "Unsupported populated schema identity/version");
    let tables = objects.iter().filter(|(_,kind,_,_)| kind == "table").map(|(name,_,_,_)| name.as_str()).collect::<std::collections::HashSet<_>>();
    let count = tables.iter().filter(|name| task_schema_name(name)).count();
    let profile = match (version, count) {
        (1,0) => SchemaProfile::PreTask,
        (1,6) if !tables.contains("task_safety_version") => SchemaProfile::InitialTask,
        (1 | 2,7) => SchemaProfile::Core,
        (3,9) => SchemaProfile::Scheduled,
        (4,9) => SchemaProfile::Current,
        _ => anyhow::bail!("Unsupported partial task schema"),
    };
    let mut names = std::collections::HashSet::new();
    for sql in LocalStore::SCHEMA.split(';').filter(|s| !s.trim().is_empty()) {
        let mut required = schema_tokens(sql)?;
        let unique = required.get(1).map(String::as_str) == Some("unique");
        let kind = required[if unique { 2 } else { 1 }].clone();
        let name = required[if unique { 3 } else { 2 }].clone();
        names.insert(name.clone());
        let omitted = task_schema_name(&name) && (profile == SchemaProfile::PreTask || (matches!(profile,SchemaProfile::InitialTask | SchemaProfile::Core) && name.starts_with("routine_schedule_")) || (profile == SchemaProfile::InitialTask && name == "task_safety_version"));
        if omitted { anyhow::ensure!(!objects.iter().any(|(n,_,_,_)| n == &name), "Unexpected predecessor object {name}"); continue; }
        let actual = objects.iter().find(|(n,_,_,_)| n == &name).ok_or_else(|| anyhow::anyhow!("Missing schema object {name}"))?;
        anyhow::ensure!(actual.1 == kind, "Wrong schema object type {name}");
        if profile == SchemaProfile::InitialTask && name == "task_authority" {
            let at = required.iter().position(|t| t == "admission_floor").ok_or_else(|| anyhow::anyhow!("Invalid original authority schema"))?;
            required.drain(at-1..at+2);
        }
        anyhow::ensure!(schema_tokens(actual.3.as_deref().ok_or_else(|| anyhow::anyhow!("Missing schema SQL"))?)? == required, "Mismatched schema definition {name}");
    }
    for (name,_,table,_) in &objects {
        if task_schema_name(name) || task_schema_name(table) { anyhow::ensure!(names.contains(name), "Unexpected task schema object {name}"); }
    }
    if tables.contains("task_safety_version") {
        let invalid: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM task_safety_version WHERE id IS NOT 1 OR version IS NOT 1)", [], |r| r.get(0))?;
        anyhow::ensure!(!invalid, "Invalid task safety version record");
    }
    crate::artifacts::validate_proposal_artifact_schema(conn, profile == SchemaProfile::Current)
        .map_err(|error| anyhow::anyhow!("Artifact schema validation failed: {error:?}"))?;
    Ok(profile)
}

fn install_local_schema_tx(tx: &Transaction<'_>, profile: SchemaProfile) -> anyhow::Result<()> {
    let synchronous: i64 = tx.pragma_query_value(None,"synchronous",|r|r.get(0))?;
    let fullfsync: i64 = tx.pragma_query_value(None,"fullfsync",|r|r.get(0))?;
    let checkpoint: i64 = tx.pragma_query_value(None,"checkpoint_fullfsync",|r|r.get(0))?;
    anyhow::ensure!(synchronous == 2 && fullfsync == 1 && checkpoint == 1,"Installer durability is unavailable");
    if profile == SchemaProfile::InitialTask { tx.execute_batch("ALTER TABLE task_authority ADD COLUMN admission_floor REAL;")?; }
    if profile != SchemaProfile::Current { tx.execute_batch(LocalStore::SCHEMA)?; }
    if profile != SchemaProfile::Current {
        crate::artifacts::install_proposal_artifact_schema_tx(tx)
            .map_err(|error| anyhow::anyhow!("Artifact schema installation failed: {error:?}"))?;
    }
    if profile == SchemaProfile::Empty { tx.pragma_update(None, "application_id", crate::config::SQLITE_APPLICATION_ID)?; }
    anyhow::ensure!(classify_schema_version(tx, 4)? == SchemaProfile::Current, "Installed schema mismatch");
    if profile != SchemaProfile::Current { tx.pragma_update(None, "user_version", 4)?; }
    Ok(())
}

pub struct LocalStore {
    pub(crate) connection: Mutex<Connection>,
}

pub struct Upsert {
    pub previous: Option<Message>,
    pub changed: bool,
}

pub struct MessageSearchHit {
    pub chat_id: String,
    pub message_id: String,
    pub snippet: String,
    pub author: Author,
    pub created_at: f64,
}

/// Captured authority for one execution. Fields are host-issued, never model metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskLease {
    pub(crate) account_epoch: String,
    pub(crate) owner_epoch: String,
    pub(crate) task_id: String,
    pub(crate) execution_id: String,
    pub(crate) incarnation: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct ScheduleLease {
    pub revision: String,
    pub binding: String,
    pub occurrence: i64,
}

pub(crate) fn schedule_binding(state: &State, routine: &crate::model::Routine) -> anyhow::Result<String> {
    let bot = state.bots.iter().find(|b| b.id == routine.bot_id);
    let dm = state.chats.iter().find(|c| c.meta.kind == "dm" && c.meta.bot_ids.contains(&routine.bot_id));
    Ok(serde_json::to_string(&(routine.id.as_str(), routine.bot_id.as_str(), &routine.name, &routine.prompt, &routine.schedule, routine.is_enabled, routine.enabled_at, &routine.check, bot.map(|b| &b.runner_id), dm.map(|c| (&c.meta.id, &c.meta.bot_ids)), state.paused, &state.pause_version))?)
}

fn schedule_unresolved_tx(tx: &Transaction<'_>, epoch: &str, routine: &str) -> anyhow::Result<bool> {
    Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM local_tasks t WHERE account_epoch=?1 AND routine_id=?2 AND (state IN ('queued','running') OR (state IN ('interrupted','needs_review') AND resolved_at IS NULL) OR EXISTS(SELECT 1 FROM task_effect_receipts r WHERE r.account_epoch=t.account_epoch AND r.task_id=t.task_id AND r.state IN ('started','unknown'))))", params![epoch,routine], |r| r.get(0))?)
}

fn consume_schedule_tx(tx: &Transaction<'_>, lease: &TaskLease, routine: &str, schedule: &ScheduleLease, next: i64) -> anyhow::Result<()> {
    check_authority_tx(tx, lease)?;
    anyhow::ensure!(!schedule_unresolved_tx(tx,&lease.account_epoch,routine)?, "Routine has pending or unknown intent");
    anyhow::ensure!(next > schedule.occurrence, "Invalid next occurrence");
    let changed=tx.execute("UPDATE routine_schedule_authority SET next_at=?6 WHERE account_epoch=?1 AND routine_id=?2 AND revision=?3 AND binding=?4 AND next_at=?5 AND revoked=0",params![lease.account_epoch,routine,schedule.revision,schedule.binding,schedule.occurrence,next])?;
    anyhow::ensure!(changed==1,"Schedule authority changed or occurrence consumed");
    tx.execute("INSERT INTO routine_schedule_occurrences VALUES(?1,?2,?3,?4)",params![lease.account_epoch,routine,schedule.revision,schedule.occurrence])?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState { Queued, Running, Finished, Interrupted, NeedsReview }

impl TaskState {
    fn parse(value: &str) -> anyhow::Result<Self> {
        Ok(match value {
            "queued" => Self::Queued, "running" => Self::Running, "finished" => Self::Finished,
            "interrupted" => Self::Interrupted, "needs_review" => Self::NeedsReview,
            _ => anyhow::bail!("Invalid persisted task state"),
        })
    }
}

/// Immutable effective invocation binding. Digest construction belongs to the execution host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvocationBinding {
    pub invocation_id: String,
    pub parent_invocation_id: Option<String>,
    pub ordinal: u64,
    pub attempt_id: String,
    pub receipt_id: String,
    pub revision: u64,
    pub digest: [u8; 32],
}

/// Persisted state of one frozen invocation. Unknown stored text is an error, never a guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InvocationState { Pending, Authorized, Denied, Dismissed, Expired, Admitted }

impl InvocationState {
    fn parse(text: &str) -> anyhow::Result<Self> {
        Ok(match text {
            "pending" => Self::Pending, "authorized" => Self::Authorized, "denied" => Self::Denied,
            "dismissed" => Self::Dismissed, "expired" => Self::Expired, "admitted" => Self::Admitted,
            _ => anyhow::bail!("Invalid persisted invocation state"),
        })
    }
}

/// The outcome a durable proposal origin already carries. `Saved` is never produced here:
/// only a live, acknowledged save produces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposalSaveOutcome { Saved, AlreadySaved, NotDispatched, Unknown }

/// A frozen invocation, either minted by this call or loaded from the retained row. `created`
/// false means the host must use `binding` as returned and discard its own candidate.
#[derive(Clone, Debug)]
pub(crate) struct FrozenInvocation { pub binding: InvocationBinding, pub state: InvocationState, pub created: bool }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationDecision { Authorized, Denied, Dismissed, Expired }

impl AuthorizationDecision {
    fn as_str(self) -> &'static str {
        match self { Self::Authorized => "authorized", Self::Denied => "denied", Self::Dismissed => "dismissed", Self::Expired => "expired" }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationKind { UserOnce, UserAlways, Rule, ReviewAllow, NotReviewed }

impl AuthorizationKind {
    fn as_str(self) -> &'static str {
        match self { Self::UserOnce => "user_once", Self::UserAlways => "user_always", Self::Rule => "rule", Self::ReviewAllow => "review_allow", Self::NotReviewed => "not_reviewed" }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptOutcome { Finished, DefinitelyNotSent, Unknown }

impl ReceiptOutcome {
    fn as_str(self) -> &'static str {
        match self { Self::Finished => "finished", Self::DefinitelyNotSent => "failed", Self::Unknown => "unknown" }
    }
}

fn structural_id(id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')), "Invalid structural identity");
    Ok(())
}

fn validate_binding(binding: &InvocationBinding) -> anyhow::Result<()> {
    for id in [&binding.invocation_id, &binding.attempt_id, &binding.receipt_id] { structural_id(id)?; }
    if let Some(id) = &binding.parent_invocation_id { structural_id(id)?; }
    anyhow::ensure!(binding.ordinal <= i64::MAX as u64 && binding.revision <= i64::MAX as u64, "Invocation identity overflow");
    Ok(())
}

fn check_authority_tx(tx: &Transaction<'_>, lease: &TaskLease) -> anyhow::Result<()> {
    let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM task_authority WHERE id=1 AND account_epoch=?1 AND owner_epoch=?2 AND closed=0)", params![lease.account_epoch,lease.owner_epoch], |r| r.get(0))?;
    anyhow::ensure!(valid, "Stale execution authority");
    Ok(())
}

fn check_running_tx(tx: &Transaction<'_>, lease: &TaskLease) -> anyhow::Result<()> {
    check_authority_tx(tx, lease)?;
    let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM local_tasks WHERE account_epoch=?1 AND task_id=?2 AND owner_epoch=?3 AND execution_id=?4 AND state='running' AND cancel_requested=0)", params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id], |r| r.get(0))?;
    anyhow::ensure!(valid, "Task is not admitted for effects");
    Ok(())
}

/// Caller owns the safety transaction and the protected host policy recheck.
/// A true result is dispatch permission only after the caller acknowledges COMMIT.
pub(crate) fn admit_invocation_tx(tx: &Transaction<'_>, lease: &TaskLease, binding: &InvocationBinding) -> anyhow::Result<bool> {
    validate_binding(binding)?;
    check_running_tx(tx, lease)?;
    let changed = tx.execute("UPDATE task_invocations SET state='admitted' WHERE account_epoch=?1 AND task_id=?2 AND owner_epoch=?3 AND execution_id=?4 AND invocation_id=?5 AND parent_id IS ?6 AND ordinal=?7 AND attempt_id=?8 AND receipt_id=?9 AND revision=?10 AND digest=?11 AND state='authorized'",
        params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id,binding.invocation_id,binding.parent_invocation_id,binding.ordinal,binding.attempt_id,binding.receipt_id,binding.revision,binding.digest.as_slice()])?;
    if changed == 0 { return Ok(false); }
    tx.execute("INSERT INTO task_effect_receipts VALUES(?1,?2,?3,?4,?5,?6,'started')",
        params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id,binding.attempt_id,binding.receipt_id])?;
    Ok(true)
}

/// Caller owns the safety transaction and retains the admitted invocation binding.
/// A true result is durable completion only after the caller acknowledges COMMIT.
pub(crate) fn finish_receipt_tx(tx: &Transaction<'_>, lease: &TaskLease, binding: &InvocationBinding, outcome: ReceiptOutcome) -> anyhow::Result<bool> {
    validate_binding(binding)?;
    check_authority_tx(tx, lease)?;
    Ok(tx.execute("UPDATE task_effect_receipts SET state=?7 WHERE account_epoch=?1 AND task_id=?2 AND owner_epoch=?3 AND execution_id=?4 AND attempt_id=?5 AND receipt_id=?6 AND state='started' AND EXISTS(SELECT 1 FROM task_invocations i WHERE i.account_epoch=?1 AND i.task_id=?2 AND i.owner_epoch=?3 AND i.execution_id=?4 AND i.attempt_id=?5 AND i.receipt_id=?6 AND i.invocation_id=?8 AND i.revision=?9 AND i.digest=?10 AND i.state='admitted')",
        params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id,binding.attempt_id,binding.receipt_id,outcome.as_str(),binding.invocation_id,binding.revision,binding.digest.as_slice()])? == 1)
}

/// Retained binding read through the caller's connection or transaction, so a freeze inside
/// one safety transaction never re-enters the non-reentrant connection mutex.
pub(crate) fn load_invocation_tx(conn: &Connection, lease: &TaskLease, invocation_id: &str) -> anyhow::Result<Option<(InvocationBinding, InvocationState)>> {
    structural_id(invocation_id)?;
    let row = conn.query_row(
        "SELECT parent_id,ordinal,attempt_id,receipt_id,revision,digest,state FROM task_invocations WHERE account_epoch=?1 AND task_id=?2 AND owner_epoch=?3 AND execution_id=?4 AND invocation_id=?5",
        params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id,invocation_id],
        |row| Ok((row.get::<_,Option<String>>(0)?,row.get::<_,i64>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,i64>(4)?,row.get::<_,Vec<u8>>(5)?,row.get::<_,String>(6)?))).optional()?;
    let Some((parent, ordinal, attempt_id, receipt_id, revision, digest, state)) = row else { return Ok(None) };
    let digest: [u8;32] = digest.try_into().map_err(|_| anyhow::anyhow!("Invalid persisted invocation digest"))?;
    anyhow::ensure!(ordinal >= 0 && revision >= 0, "Invalid persisted invocation identity");
    let binding = InvocationBinding { invocation_id: invocation_id.to_string(), parent_invocation_id: parent, ordinal: ordinal as u64, attempt_id, receipt_id, revision: revision as u64, digest };
    validate_binding(&binding)?;
    Ok(Some((binding, InvocationState::parse(&state)?)))
}

fn upsert_message_tx(tx: &Transaction<'_>, message: &Message) -> anyhow::Result<Upsert> {
    let json = serde_json::to_string(message)?;
    let existing: Option<(String, String, i64)> = tx.query_row("SELECT message_json, chat_id, position FROM messages WHERE id=?1", [&message.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let previous = existing.as_ref().map(|(stored,_,_)|serde_json::from_str(stored)).transpose()?;
    if existing.as_ref().is_some_and(|(stored,_,_)|stored==&json) { return Ok(Upsert { previous, changed:false }); }
    let position = match existing.as_ref() {
        Some((_,chat,position)) if chat==&message.chat_id => *position,
        _ => tx.query_row("SELECT COALESCE(MAX(position),0)+1 FROM messages WHERE chat_id=?1",[&message.chat_id],|r|r.get(0))?,
    };
    let (author_kind,author_bot_id)=author_columns(&message.author);
    tx.execute("INSERT INTO messages(id,chat_id,position,sort_at,created_at,author_kind,author_bot_id,body_kind,is_complete,text_nonempty,message_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(id) DO UPDATE SET chat_id=excluded.chat_id,position=excluded.position,sort_at=excluded.sort_at,created_at=excluded.created_at,author_kind=excluded.author_kind,author_bot_id=excluded.author_bot_id,body_kind=excluded.body_kind,is_complete=excluded.is_complete,text_nonempty=excluded.text_nonempty,message_json=excluded.message_json",
        params![message.id,message.chat_id,position,message.promoted_at.unwrap_or(message.created_at),message.created_at,author_kind,author_bot_id,body_kind(&message.body),message.is_complete(),matches!(&message.body,Body::Text{text,..} if !text.trim().is_empty()),json])?;
    Ok(Upsert {previous,changed:true})
}

fn queue_task_tx(tx: &Transaction<'_>, lease: &TaskLease, job: &crate::model::Job, local_intent: bool) -> anyhow::Result<bool> {
    check_authority_tx(tx, lease)?;
    for id in [&lease.task_id,&lease.execution_id,&job.chat_id,&job.bot_id] { structural_id(id)?; }
    anyhow::ensure!(lease.task_id == job.id, "Task identity differs from Job.id");
    if let Some(id) = &job.routine_id { structural_id(id)?; }
    // A migrated home lacks a complete historical Job inventory. Sender timestamps
    // cannot establish new intent: all ambiguous admissions stay closed.
    let legacy_closed: bool = tx.query_row("SELECT admission_floor IS NOT NULL FROM task_authority WHERE id=1",[],|r|r.get(0))?;
    if legacy_closed && !local_intent {
        tx.execute("INSERT OR IGNORE INTO task_fences VALUES(?1,?2)",params![lease.account_epoch,lease.task_id])?;
        return Ok(false);
    }
    let fresh = tx.execute("INSERT INTO task_fences VALUES(?1,?2) ON CONFLICT DO NOTHING", params![lease.account_epoch,lease.task_id])? == 1;
    if !fresh { return Ok(false); }
    tx.execute("INSERT INTO local_tasks(account_epoch,task_id,owner_epoch,execution_id,chat_id,bot_id,routine_id,state) VALUES(?1,?2,?3,?4,?5,?6,?7,'queued')", params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id,job.chat_id,job.bot_id,job.routine_id])?;
    Ok(true)
}

impl LocalStore {
    pub(crate) fn reauthorize_schedule(&self, lease: &TaskLease, routine: &str, binding: &str, next: i64) -> anyhow::Result<()> {
        self.safety(|tx| {
            check_authority_tx(tx,lease)?;
            anyhow::ensure!(!schedule_unresolved_tx(tx,&lease.account_epoch,routine)?,"Routine has pending or unknown intent");
            tx.execute("INSERT INTO routine_schedule_authority VALUES(?1,?2,?3,?4,?5,0) ON CONFLICT(account_epoch,routine_id) DO UPDATE SET revision=excluded.revision,binding=excluded.binding,next_at=excluded.next_at,revoked=0",params![lease.account_epoch,routine,uuid::Uuid::new_v4().to_string(),binding,next])?;
            Ok(())
        })
    }
    pub(crate) fn revoke_schedule(&self, routine: &str) -> anyhow::Result<()> {
        self.safety(|tx| { tx.execute("UPDATE routine_schedule_authority SET revoked=1 WHERE routine_id=?1",[routine])?; Ok(()) })
    }
    pub(crate) fn reconcile_schedules(&self, state: &State) -> anyhow::Result<()> {
        self.safety(|tx| reconcile_schedules_tx(tx,state))
    }
    pub(crate) fn schedule_lease(&self, routine: &str, binding: &str, now: i64, next: i64) -> anyhow::Result<Option<ScheduleLease>> {
        self.safety(|tx| {
            // Skip elapsed occurrences, never dispatch a missed-tick backlog.
            tx.execute("UPDATE routine_schedule_authority SET next_at=?4 WHERE routine_id=?1 AND binding=?2 AND revoked=0 AND next_at<?3",params![routine,binding,now.saturating_sub(30),next])?;
            Ok(tx.query_row("SELECT revision,binding,next_at FROM routine_schedule_authority WHERE routine_id=?1 AND binding=?2 AND revoked=0 AND next_at<=?3 AND account_epoch=(SELECT account_epoch FROM task_authority WHERE id=1 AND closed=0)",params![routine,binding,now],|r|Ok(ScheduleLease{revision:r.get(0)?,binding:r.get(1)?,occurrence:r.get(2)?})).optional()?)
        })
    }
    pub(crate) fn commit_scheduled_occurrence(&self, lease: &TaskLease, job: Option<&crate::model::Job>, routine: &str, schedule: &ScheduleLease, next: i64, chat: &str, bot: &str, at: i64, set: &std::collections::BTreeMap<String,serde_json::Value>, delete: &[String]) -> anyhow::Result<()> {
        self.safety(|tx| {
            consume_schedule_tx(tx,lease,routine,schedule,next)?;
            if let Some(job)=job {
                anyhow::ensure!(queue_task_tx(tx,lease,job,true)?,"Routine task already fenced");
                tx.execute("UPDATE local_tasks SET check_report=?3 WHERE account_epoch=?1 AND task_id=?2",params![lease.account_epoch,lease.task_id,serde_json::to_string(&job.check)?])?;
            }
            for key in delete { tx.execute("DELETE FROM codemode_store WHERE chat_id=?1 AND bot_id=?2 AND key=?3",params![chat,bot,key])?; }
            for (key,value) in set { tx.execute("INSERT INTO codemode_store VALUES(?1,?2,?3,?4) ON CONFLICT(chat_id,bot_id,key) DO UPDATE SET json=excluded.json",params![chat,bot,key,serde_json::to_string(value)?])?; }
            tx.execute("INSERT INTO routine_check_commits VALUES(?1,?2,?3) ON CONFLICT(account_epoch,routine_id) DO UPDATE SET checked_at=excluded.checked_at",params![lease.account_epoch,routine,at])?;
            Ok(())
        })
    }
    pub(crate) fn commit_submission(&self, message: &Message, tasks: &[(crate::model::Job, TaskLease)], outbox: &[OutboxItem], finalize: impl FnOnce() -> anyhow::Result<()>) -> anyhow::Result<Upsert> {
        self.safety(|tx| {
            for (job,lease) in tasks { anyhow::ensure!(queue_task_tx(tx,lease,job,true)?, "Submission task already fenced"); }
            let result=upsert_message_tx(tx,message)?;
            for item in outbox { queue_outbox_tx(tx,item)?; }
            finalize()?;
            Ok(result)
        })
    }
    pub(crate) fn legacy_execution_closed(&self) -> anyhow::Result<bool> {
        Ok(self.connection.lock().unwrap().query_row("SELECT admission_floor IS NOT NULL FROM task_authority WHERE id=1",[],|r|r.get(0)).optional()?.unwrap_or(false))
    }
    pub(crate) fn history_task_lease(&self, owner: &str, task: &str, incarnation: u64) -> anyhow::Result<Option<TaskLease>> {
        Ok(self.connection.lock().unwrap().query_row("SELECT account_epoch,owner_epoch,execution_id FROM local_tasks WHERE task_id=?1 AND state IN ('finished','interrupted','needs_review') AND account_epoch=(SELECT account_epoch FROM task_authority WHERE id=1 AND owner_epoch=?2 AND closed=0)",params![task,owner],|r|Ok(TaskLease {account_epoch:r.get(0)?,owner_epoch:r.get(1)?,execution_id:r.get(2)?,task_id:task.into(),incarnation})).optional()?)
    }
    pub(crate) fn active_task_lease(&self, owner: &str, task: &str, incarnation: u64) -> anyhow::Result<Option<TaskLease>> {
        Ok(self.connection.lock().unwrap().query_row("SELECT account_epoch,execution_id FROM local_tasks WHERE owner_epoch=?1 AND task_id=?2 AND state='running' AND account_epoch=(SELECT account_epoch FROM task_authority WHERE id=1 AND owner_epoch=?1 AND closed=0)",params![owner,task],|r| Ok(TaskLease { account_epoch:r.get(0)?,execution_id:r.get(1)?,owner_epoch:owner.into(),task_id:task.into(),incarnation })).optional()?)
    }
    pub(crate) fn commit_routine_check(&self, lease: &TaskLease, job: Option<&crate::model::Job>, routine: &str, chat: &str, bot: &str, at: i64, set: &std::collections::BTreeMap<String,serde_json::Value>, delete: &[String]) -> anyhow::Result<bool> {
        self.safety(|tx| {
            check_authority_tx(tx,lease)?;
            if let Some(job) = job {
                anyhow::ensure!(queue_task_tx(tx,lease,job,false)?, "Routine task already exists");
                tx.execute("UPDATE local_tasks SET check_report=?3 WHERE account_epoch=?1 AND task_id=?2",params![lease.account_epoch,lease.task_id,serde_json::to_string(&job.check)?])?;
            }
            for key in delete { tx.execute("DELETE FROM codemode_store WHERE chat_id=?1 AND bot_id=?2 AND key=?3",params![chat,bot,key])?; }
            for (key,value) in set { tx.execute("INSERT INTO codemode_store VALUES(?1,?2,?3,?4) ON CONFLICT(chat_id,bot_id,key) DO UPDATE SET json=excluded.json",params![chat,bot,key,serde_json::to_string(value)?])?; }
            tx.execute("INSERT INTO routine_check_commits VALUES(?1,?2,?3) ON CONFLICT(account_epoch,routine_id) DO UPDATE SET checked_at=excluded.checked_at",params![lease.account_epoch,routine,at])?;
            Ok(true)
        })
    }

    pub(crate) fn routine_checked_at(&self, routine: &str) -> anyhow::Result<Option<i64>> {
        Ok(self.connection.lock().unwrap().query_row("SELECT checked_at FROM routine_check_commits WHERE routine_id=?1 AND account_epoch=(SELECT account_epoch FROM task_authority WHERE id=1 AND closed=0)",[routine],|r|r.get(0)).optional()?)
    }
    pub(crate) fn current_task_account_epoch(&self) -> anyhow::Result<Option<String>> {
        Ok(self.connection.lock().unwrap().query_row("SELECT account_epoch FROM task_authority WHERE id=1 AND closed=0", [], |r| r.get(0)).optional()?)
    }

    pub(crate) fn task_account_epoch(&self, owner: &str) -> anyhow::Result<String> {
        self.safety(|tx| {
            let authority: Option<(String,String,bool)> = tx.query_row("SELECT account_epoch,owner_epoch,closed FROM task_authority WHERE id=1", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            match authority {
                Some((epoch,current,false)) if current == owner => Ok(epoch),
                Some(_) => anyhow::bail!("Stale task owner"),
                None => {
                    let epoch = uuid::Uuid::new_v4().to_string();
                    tx.execute("INSERT INTO task_authority(id,account_epoch,owner_epoch,closed) VALUES(1,?1,?2,0)", params![epoch,owner])?;
                    tx.execute("INSERT INTO task_safety_version VALUES(1,1)",[])?;
                    Ok(epoch)
                }
            }
        })
    }
    /// All safety mutations use this boundary. Keep FULL for the connection lifetime so
    /// unrelated state/policy writes cannot weaken a committed execution fence.
    fn safety<T>(&self, write: impl FnOnce(&Transaction<'_>) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let mut connection = self.connection.lock().unwrap();
        let synchronous: i64 = connection.pragma_query_value(None, "synchronous", |row| row.get(0))?;
        let fullfsync: i64 = connection.pragma_query_value(None, "fullfsync", |row| row.get(0))?;
        anyhow::ensure!(synchronous == 2 && fullfsync == 1, "Task durability is unavailable");
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let value = write(&tx)?;
        tx.commit()?;
        Ok(value)
    }

    /// Caller holds the exclusive home lock. Legacy journals become replay-denial
    /// evidence, never executable tasks; existing account data stays byte-for-byte intact.
    pub(crate) fn recover_task_owner(&self, owner: &str) -> anyhow::Result<String> {
        structural_id(owner)?;
        self.safety(|tx| {
            let authority: Option<(String, bool)> = tx.query_row(
                "SELECT account_epoch,closed FROM task_authority WHERE id=1", [], |r| Ok((r.get(0)?, r.get(1)?)),
            ).optional()?;
            let epoch = match authority {
                Some((epoch, false)) => epoch,
                Some((_, true)) => anyhow::bail!("Task account is closed"),
                None => {
                    let orphaned: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM local_tasks UNION ALL SELECT 1 FROM task_fences UNION ALL SELECT 1 FROM task_invocations UNION ALL SELECT 1 FROM task_effect_receipts)", [], |r| r.get(0))?;
                    anyhow::ensure!(!orphaned, "Task authority is missing from a store containing safety records");
                    let migrated: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM task_safety_version)",[],|r|r.get(0))?;
                    anyhow::ensure!(!migrated, "Migrated task authority is missing; automatic reconstruction is forbidden");
                    let epoch = uuid::Uuid::new_v4().to_string();
                    let legacy: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM bots UNION ALL SELECT 1 FROM messages UNION ALL SELECT 1 FROM sent_jobs UNION ALL SELECT 1 FROM memory_turn_admissions)",[],|r|r.get(0))?;
                    let floor = legacy.then_some(1.0);
                    tx.execute("INSERT INTO task_authority(id,account_epoch,owner_epoch,closed,admission_floor) VALUES(1,?1,?2,0,?3)", params![epoch, owner,floor])?;
                    tx.execute("INSERT INTO task_safety_version VALUES(1,1)",[])?;
                    // Available legacy Job identities are denial evidence only. Sent jobs
                    // remain pending remote-result waits; migration never dispatches them.
                    tx.execute("INSERT OR IGNORE INTO task_fences SELECT ?1,id FROM sent_jobs", [&epoch])?;
                    tx.execute("INSERT OR IGNORE INTO task_fences SELECT ?1,job_id FROM memory_turn_admissions", [&epoch])?;
                    let turns: Vec<String> = tx.prepare("SELECT json FROM device_turns")?
                        .query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                    for json in turns {
                        let turns: Vec<LiveTurn> = serde_json::from_str(&json).context("Invalid legacy turn journal")?;
                        for turn in turns {
                            structural_id(&turn.job_id)?;
                            tx.execute("INSERT OR IGNORE INTO task_fences VALUES(?1,?2)",params![epoch,turn.job_id])?;
                        }
                    }
                    epoch
                }
            };
            tx.execute("UPDATE task_effect_receipts SET state='unknown' WHERE account_epoch=?1 AND state='started'", [&epoch])?;
            tx.execute("UPDATE task_invocations SET state='dismissed' WHERE account_epoch=?1 AND state IN ('pending','authorized')", [&epoch])?;
            tx.execute("UPDATE local_tasks SET state=CASE WHEN EXISTS(SELECT 1 FROM task_effect_receipts r WHERE r.account_epoch=local_tasks.account_epoch AND r.task_id=local_tasks.task_id) THEN 'needs_review' ELSE 'interrupted' END WHERE account_epoch=?1 AND state IN ('queued','running')", [&epoch])?;
            tx.execute("UPDATE task_authority SET owner_epoch=?1 WHERE id=1 AND account_epoch=?2 AND closed=0", params![owner, epoch])?;
            Ok(epoch)
        })
    }

    pub(crate) fn queue_task(&self, lease: &TaskLease, job: &crate::model::Job) -> anyhow::Result<bool> {
        self.safety(|tx| queue_task_tx(tx, lease, job, false))
    }

    /// Only the local submission boundary calls this with a newly server-minted Job id.
    pub(crate) fn queue_local_intent(&self, lease: &TaskLease, job: &crate::model::Job) -> anyhow::Result<bool> {
        self.safety(|tx| queue_task_tx(tx, lease, job, true))
    }

    pub(crate) fn start_task(&self, lease: &TaskLease) -> anyhow::Result<bool> {
        self.safety(|tx| {
            check_authority_tx(tx, lease)?;
            Ok(tx.execute("UPDATE local_tasks SET state='running' WHERE account_epoch=?1 AND task_id=?2 AND owner_epoch=?3 AND execution_id=?4 AND state='queued' AND cancel_requested=0",
                params![lease.account_epoch, lease.task_id, lease.owner_epoch, lease.execution_id])? == 1)
        })
    }

    pub(crate) fn finish_task(&self, lease: &TaskLease, interrupted: bool) -> anyhow::Result<bool> {
        self.safety(|tx| {
            check_authority_tx(tx, lease)?;
            Ok(tx.execute("UPDATE local_tasks SET state=CASE WHEN EXISTS(SELECT 1 FROM task_effect_receipts r WHERE r.account_epoch=local_tasks.account_epoch AND r.task_id=local_tasks.task_id AND r.state IN ('started','unknown')) THEN 'needs_review' WHEN cancel_requested=1 OR ?5 THEN 'interrupted' ELSE 'finished' END WHERE account_epoch=?1 AND task_id=?2 AND owner_epoch=?3 AND execution_id=?4 AND state='running'",
                params![lease.account_epoch, lease.task_id, lease.owner_epoch, lease.execution_id, interrupted])? == 1)
        })
    }

    pub fn task_state(&self, lease: &TaskLease) -> anyhow::Result<Option<TaskState>> {
        let value: Option<String> = self.connection.lock().unwrap().query_row(
            "SELECT state FROM local_tasks WHERE account_epoch=?1 AND task_id=?2 AND owner_epoch=?3 AND execution_id=?4",
            params![lease.account_epoch, lease.task_id, lease.owner_epoch, lease.execution_id], |r| r.get(0),
        ).optional()?;
        value.as_deref().map(TaskState::parse).transpose()
    }

    pub(crate) fn cancel_tasks(&self, epoch: &str, task: Option<&str>, chat: Option<&str>, bot: Option<&str>) -> anyhow::Result<()> {
        self.safety(|tx| {
            if let Some(id) = task {
                structural_id(id)?;
                tx.execute("INSERT OR IGNORE INTO task_fences(account_epoch,task_id) SELECT ?1,?2 WHERE EXISTS(SELECT 1 FROM task_authority WHERE account_epoch=?1 AND closed=0)", params![epoch,id])?;
            }
            tx.execute("UPDATE local_tasks SET cancel_requested=1 WHERE account_epoch=?1 AND (?2 IS NULL OR task_id=?2) AND (?3 IS NULL OR chat_id=?3) AND (?4 IS NULL OR bot_id=?4)", params![epoch,task,chat,bot])?;
            tx.execute("UPDATE task_invocations SET state='dismissed' WHERE account_epoch=?1 AND state IN ('pending','authorized') AND EXISTS(SELECT 1 FROM local_tasks t WHERE t.account_epoch=task_invocations.account_epoch AND t.task_id=task_invocations.task_id AND t.cancel_requested=1)", [epoch])?;
            Ok(())
        })
    }

    pub(crate) fn close_task_account(&self) -> anyhow::Result<()> {
        self.safety(|tx| {
            tx.execute("UPDATE task_authority SET closed=1 WHERE id=1", [])?;
            tx.execute("UPDATE local_tasks SET cancel_requested=1", [])?;
            tx.execute("UPDATE task_invocations SET state='dismissed' WHERE state IN ('pending','authorized')", [])?;
            Ok(())
        })
    }

    pub(crate) fn routine_needs_review(&self, epoch: &str, routine: &str) -> anyhow::Result<bool> {
        Ok(self.connection.lock().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM local_tasks WHERE account_epoch=?1 AND routine_id=?2 AND state IN ('interrupted','needs_review') AND resolved_at IS NULL)", params![epoch,routine], |r| r.get(0))?)
    }

    /// Borrowed task safety transaction, so one caller can admit its task invocation and a
    /// workspace effect belonging to it in one step. Same settings, same mutex, same IMMEDIATE
    /// scope as every other safety write; the closure owns COMMIT or rollback. No such workspace
    /// effect is admitted through it yet: `artifacts::store` exports no facade and no production
    /// path installs its tables, so today the closure carries the task admission alone.
    pub(crate) fn safety_tx<T>(&self, write: impl FnOnce(&Transaction<'_>) -> anyhow::Result<T>) -> anyhow::Result<T> {
        self.safety(write)
    }

    /// Read-only and fallible. Keyed by the full lease plus `invocation_id`. Any stored value
    /// that is not a complete, structural, 32-byte binding is an error, never a partial one.
    pub(crate) fn load_invocation(&self, lease: &TaskLease, invocation_id: &str) -> anyhow::Result<Option<(InvocationBinding, InvocationState)>> {
        structural_id(invocation_id)?;
        let connection = self.connection.lock().unwrap();
        load_invocation_tx(&connection, lease, invocation_id)
    }

    /// One safety transaction loads the retained binding, or mints one: the select and the
    /// insert are serialised by IMMEDIATE, so a parallel proposal in the same execution gets
    /// the next ordinal and a collision surfaces as Err rather than a silent false.
    pub(crate) fn freeze_or_load_invocation(&self, lease: &TaskLease, invocation_id: &str, revision: u64, digest: [u8;32], attempt_id: &str, receipt_id: &str) -> anyhow::Result<FrozenInvocation> {
        structural_id(invocation_id)?;
        self.safety(|tx| {
            check_running_tx(tx, lease)?;
            if let Some((binding, state)) = load_invocation_tx(tx, lease, invocation_id)? {
                return Ok(FrozenInvocation { binding, state, created: false });
            }
            let ordinal: i64 = tx.query_row("SELECT COALESCE(MAX(ordinal)+1,0) FROM task_invocations WHERE account_epoch=?1 AND task_id=?2 AND execution_id=?3 AND parent_id IS NULL",
                params![lease.account_epoch,lease.task_id,lease.execution_id], |row| row.get(0))?;
            anyhow::ensure!(ordinal >= 0, "Invalid proposal ordinal");
            let binding = InvocationBinding { invocation_id: invocation_id.to_string(), parent_invocation_id: None, ordinal: ordinal as u64, attempt_id: attempt_id.to_string(), receipt_id: receipt_id.to_string(), revision, digest };
            validate_binding(&binding)?;
            tx.execute("INSERT INTO task_invocations(account_epoch,task_id,owner_epoch,execution_id,invocation_id,parent_id,ordinal,attempt_id,receipt_id,revision,digest,state) VALUES(?1,?2,?3,?4,?5,NULL,?6,?7,?8,?9,?10,'pending')",
                params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id,invocation_id,ordinal,binding.attempt_id,binding.receipt_id,revision,digest.as_slice()])?;
            Ok(FrozenInvocation { binding, state: InvocationState::Pending, created: true })
        })
    }

    /// Read-only fold of every execution's evidence for one proposal origin. `None` means the
    /// display row still decides; Err is a store fault, which the caller maps to Unknown.
    pub(crate) fn proposal_origin_outcome(&self, account_epoch: &str, card_id: &str) -> anyhow::Result<Option<ProposalSaveOutcome>> {
        structural_id(card_id)?;
        let rows: Vec<(String,Option<String>)> = {
            let connection = self.connection.lock().unwrap();
            let mut statement = connection.prepare(
                "SELECT i.state,r.state FROM task_invocations i LEFT JOIN task_effect_receipts r ON r.account_epoch=i.account_epoch AND r.receipt_id=i.receipt_id WHERE i.account_epoch=?1 AND i.invocation_id=?2")?;
            let rows = statement.query_map(params![account_epoch,card_id], |row| Ok((row.get(0)?,row.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            rows
        };
        let mut outcome: Option<ProposalSaveOutcome> = None;
        for (invocation, receipt) in rows {
            let candidate = match InvocationState::parse(&invocation)? {
                InvocationState::Admitted => match receipt.as_deref() {
                    Some("finished") => ProposalSaveOutcome::AlreadySaved,
                    Some("failed") => ProposalSaveOutcome::NotDispatched,
                    Some("started") | Some("unknown") | None => ProposalSaveOutcome::Unknown,
                    Some(other) => anyhow::bail!("Invalid persisted receipt state {other}"),
                },
                InvocationState::Authorized => ProposalSaveOutcome::NotDispatched,
                InvocationState::Pending | InvocationState::Denied | InvocationState::Dismissed | InvocationState::Expired => continue,
            };
            // Fold conservatively across executions: a completed acceptance outranks an
            // ambiguity, and an ambiguity outranks a refusal that never dispatched.
            outcome = Some(match (outcome, candidate) {
                (Some(ProposalSaveOutcome::AlreadySaved), _) | (None, ProposalSaveOutcome::AlreadySaved) => ProposalSaveOutcome::AlreadySaved,
                (Some(ProposalSaveOutcome::Unknown), _) | (None, ProposalSaveOutcome::Unknown) => ProposalSaveOutcome::Unknown,
                (current, _) => current.unwrap_or(ProposalSaveOutcome::NotDispatched),
            });
        }
        Ok(outcome)
    }

    /// Freeze once; changing a binding is a different invocation, not an UPDATE.
    pub(crate) fn insert_invocation(&self, lease: &TaskLease, binding: &InvocationBinding) -> anyhow::Result<bool> {
        validate_binding(binding)?;
        self.safety(|tx| {
            check_running_tx(tx, lease)?;
            Ok(tx.execute("INSERT INTO task_invocations(account_epoch,task_id,owner_epoch,execution_id,invocation_id,parent_id,ordinal,attempt_id,receipt_id,revision,digest,state) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'pending') ON CONFLICT DO NOTHING",
                params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id,binding.invocation_id,binding.parent_invocation_id,binding.ordinal,binding.attempt_id,binding.receipt_id,binding.revision,binding.digest.as_slice()])? == 1)
        })
    }

    pub(crate) fn decide_invocation(&self, lease: &TaskLease, binding: &InvocationBinding, decision: AuthorizationDecision, kind: AuthorizationKind, card: Option<&str>, device: Option<&str>, at: i64) -> anyhow::Result<bool> {
        validate_binding(binding)?;
        if let Some(id) = card { structural_id(id)?; }
        if let Some(id) = device { structural_id(id)?; }
        self.safety(|tx| {
            check_running_tx(tx, lease)?;
            Ok(tx.execute("UPDATE task_invocations SET state=?12,kind=?13,card_id=?14,answering_device=?15,answered_at=?16 WHERE account_epoch=?1 AND task_id=?2 AND owner_epoch=?3 AND execution_id=?4 AND invocation_id=?5 AND parent_id IS ?6 AND ordinal=?7 AND attempt_id=?8 AND receipt_id=?9 AND revision=?10 AND digest=?11 AND state='pending'",
                params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id,binding.invocation_id,binding.parent_invocation_id,binding.ordinal,binding.attempt_id,binding.receipt_id,binding.revision,binding.digest.as_slice(),decision.as_str(),kind.as_str(),card,device,at])? == 1)
        })
    }

    /// Called only after the host's protected policy recheck; successful commit is the
    /// single dispatch permission. Any error, including ambiguous acknowledgment, forbids I/O.
    pub(crate) fn admit_invocation(&self, lease: &TaskLease, binding: &InvocationBinding) -> anyhow::Result<bool> {
        self.safety(|tx| admit_invocation_tx(tx, lease, binding))
    }

    pub(crate) fn finish_receipt(&self, lease: &TaskLease, binding: &InvocationBinding, outcome: ReceiptOutcome) -> anyhow::Result<bool> {
        self.safety(|tx| finish_receipt_tx(tx, lease, binding, outcome))
    }

    pub(crate) fn resolve_task(&self, lease: &TaskLease, at: i64) -> anyhow::Result<bool> {
        self.safety(|tx| {
            let current:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM task_authority WHERE account_epoch=?1 AND closed=0)",[&lease.account_epoch],|r|r.get(0))?;
            anyhow::ensure!(current,"Stale task account");
            Ok(tx.execute("UPDATE local_tasks SET resolved_at=?5 WHERE account_epoch=?1 AND task_id=?2 AND owner_epoch=?3 AND execution_id=?4 AND state IN ('finished','interrupted','needs_review') AND NOT EXISTS(SELECT 1 FROM task_effect_receipts r WHERE r.account_epoch=?1 AND r.task_id=?2 AND r.state='started')",
                params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id,at])? == 1)
        })
    }

    pub(crate) fn prune_tasks(&self, now: i64) -> anyhow::Result<()> {
        self.safety(|tx| {
            let cutoff = now.saturating_sub(30 * 24 * 60 * 60);
            // Routine unknown receipts remain admission evidence even after review resolution.
            tx.execute("DELETE FROM task_effect_receipts WHERE EXISTS(SELECT 1 FROM local_tasks t WHERE t.account_epoch=task_effect_receipts.account_epoch AND t.task_id=task_effect_receipts.task_id AND t.resolved_at<=?1 AND (t.routine_id IS NULL OR NOT EXISTS(SELECT 1 FROM task_effect_receipts r WHERE r.account_epoch=t.account_epoch AND r.task_id=t.task_id AND r.state='unknown')))", [cutoff])?;
            tx.execute("DELETE FROM task_invocations WHERE EXISTS(SELECT 1 FROM local_tasks t WHERE t.account_epoch=task_invocations.account_epoch AND t.task_id=task_invocations.task_id AND t.resolved_at<=?1 AND (t.routine_id IS NULL OR NOT EXISTS(SELECT 1 FROM task_effect_receipts r WHERE r.account_epoch=t.account_epoch AND r.task_id=t.task_id AND r.state='unknown')))", [cutoff])?;
            tx.execute("DELETE FROM local_tasks WHERE resolved_at<=?1 AND (routine_id IS NULL OR NOT EXISTS(SELECT 1 FROM task_effect_receipts r WHERE r.account_epoch=local_tasks.account_epoch AND r.task_id=local_tasks.task_id AND r.state='unknown'))", [cutoff])?;
            Ok(())
        })
    }
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        let config = crate::config::Config { home: parent.into(), port: 0 };
        let admission = config.ensure_home_admission()?;
        Self::open_with_admission(path, admission.as_ref(), false)
    }

    /// Deliberate staged scratch entry; ordinary App callers retain legacy admission.
    pub fn open_staged(path: &Path) -> anyhow::Result<Self> {
        let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        let config = crate::config::Config { home: parent.into(), port: 0 };
        let admission = config.staged_storage_admission()?;
        Self::open_with_admission(path, Some(&admission), true)
    }

    pub(crate) fn open_with_admission(path: &Path, admission: Option<&crate::config::StorageAdmission>, staged: bool) -> anyhow::Result<Self> {
        let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        let config = crate::config::Config { home: parent.into(), port: 0 };
        #[cfg(unix)]
        admission.ok_or_else(|| anyhow::anyhow!("Storage admission guard is required"))?.validate_target(parent)?;
        if staged {
            admission.ok_or_else(|| anyhow::anyhow!("Staged admission guard is required"))?.revalidate_staged()?;
        } else {
            config.validate_home()?;
            if let Some(admission) = admission { admission.revalidate_legacy()?; }
        }
        if path.exists() { crate::config::validate_database(path)?; }
        let mut connection = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        let profile = classify_schema(&connection)?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000; PRAGMA journal_size_limit=16777216;")?;
        #[cfg(unix)]
        admission.ok_or_else(|| anyhow::anyhow!("Storage admission guard is required"))?.validate_target(parent)?;
        if staged {
            admission.ok_or_else(|| anyhow::anyhow!("Staged admission guard is required"))?.revalidate_staged()?;
        } else {
            config.validate_home()?;
            if let Some(admission) = admission { admission.revalidate_legacy()?; }
        }
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        anyhow::ensure!(classify_schema(&tx)? == profile, "Storage schema changed during admission");
        install_local_schema_tx(&tx, profile)?;
        tx.commit()?;
        crate::config::set_private(path)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
    const SCHEMA: &'static str = "CREATE TABLE IF NOT EXISTS metadata (
                 id                       INTEGER PRIMARY KEY CHECK (id = 1),
                 auto_review_json         TEXT NOT NULL,
                 last_seq                 INTEGER NOT NULL,
                 roster_slot_seq          INTEGER NOT NULL DEFAULT 0,
                 machine_blob_hash        TEXT,
                 credentials_uploaded     INTEGER NOT NULL,
                 paused                   INTEGER NOT NULL DEFAULT 0,
                 policy_json              TEXT NOT NULL DEFAULT '{}'
             );
             CREATE TABLE IF NOT EXISTS devices (
                 id       TEXT PRIMARY KEY NOT NULL,
                 position INTEGER NOT NULL,
                 json     TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS bots (
                 id       TEXT PRIMARY KEY NOT NULL,
                 position INTEGER NOT NULL,
                 json     TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS chats (
                 id       TEXT PRIMARY KEY NOT NULL,
                 position INTEGER NOT NULL,
                 json     TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS routines (
                 id       TEXT PRIMARY KEY NOT NULL,
                 position INTEGER NOT NULL,
                 json     TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS group_deletes (
                 id       TEXT PRIMARY KEY NOT NULL,
                 position INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS pending_chat_creates (
                 id TEXT PRIMARY KEY NOT NULL,
                 original_json TEXT
             );
             CREATE TABLE IF NOT EXISTS roster_baseline (
                 name TEXT PRIMARY KEY NOT NULL,
                 seq INTEGER NOT NULL,
                 json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS blob_deletes (
                 id       TEXT PRIMARY KEY NOT NULL,
                 position INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS device_seen (
                 id      TEXT PRIMARY KEY NOT NULL,
                 seen_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS applied_blobs (
                 id       TEXT PRIMARY KEY NOT NULL,
                 position INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS chat_history (
                 chat_id      TEXT PRIMARY KEY NOT NULL,
                 before_place INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS messages (
                 id            TEXT PRIMARY KEY NOT NULL,
                 chat_id       TEXT NOT NULL,
                 position      INTEGER NOT NULL,
                 sort_at       REAL NOT NULL,
                 created_at    REAL NOT NULL,
                 author_kind   TEXT NOT NULL,
                 author_bot_id TEXT,
                 body_kind     TEXT NOT NULL,
                 is_complete   INTEGER NOT NULL,
                 text_nonempty INTEGER NOT NULL,
                 message_json  TEXT NOT NULL,
                 UNIQUE(chat_id, position)
             );
             CREATE INDEX IF NOT EXISTS messages_chat_position
                 ON messages(chat_id, position);
             CREATE INDEX IF NOT EXISTS messages_chat_order
                 ON messages(chat_id, sort_at, position);
             CREATE INDEX IF NOT EXISTS messages_author
                 ON messages(author_kind, author_bot_id, created_at);
             CREATE TABLE IF NOT EXISTS outbox (
                 position        INTEGER PRIMARY KEY AUTOINCREMENT,
                 id              TEXT UNIQUE NOT NULL,
                 kind            TEXT NOT NULL,
                 recipient       TEXT,
                 ciphertext      BLOB NOT NULL,
                 slot_name       TEXT,
                 slot_keep_first INTEGER NOT NULL DEFAULT 0,
                 group_name      TEXT
             );
             CREATE UNIQUE INDEX IF NOT EXISTS outbox_slot
                 ON outbox(slot_name) WHERE slot_name IS NOT NULL;
             CREATE TABLE IF NOT EXISTS codemode_store (
                 chat_id TEXT NOT NULL,
                 bot_id  TEXT NOT NULL,
                 key     TEXT NOT NULL,
                 json    TEXT NOT NULL,
                 PRIMARY KEY (chat_id, bot_id, key)
             );
             CREATE TABLE IF NOT EXISTS device_turns (
                 id   TEXT PRIMARY KEY NOT NULL,
                 json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sent_jobs (
                 id         TEXT PRIMARY KEY NOT NULL,
                 chat_id    TEXT NOT NULL,
                 bot_id     TEXT NOT NULL,
                 routine_id TEXT,
                 runner_id  TEXT NOT NULL,
                 sent_at    REAL NOT NULL
             );
             CREATE TABLE IF NOT EXISTS memory_config (
                 id INTEGER PRIMARY KEY CHECK(id=1), ciphertext BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS memory_deliveries (
                 id TEXT PRIMARY KEY NOT NULL, bot_id TEXT NOT NULL,
                 state TEXT NOT NULL, json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS memory_deliveries_bot ON memory_deliveries(bot_id,state);
             CREATE TABLE IF NOT EXISTS memory_fences (
                 bot_id TEXT PRIMARY KEY NOT NULL, json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS memory_runner_bindings (
                 key TEXT PRIMARY KEY NOT NULL, json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS memory_turn_admissions (
                 job_id TEXT PRIMARY KEY NOT NULL, json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS task_authority (
                 id INTEGER PRIMARY KEY CHECK(id=1), account_epoch TEXT NOT NULL,
                 owner_epoch TEXT NOT NULL, closed INTEGER NOT NULL CHECK(closed IN (0,1)), admission_floor REAL);
             CREATE TABLE IF NOT EXISTS task_safety_version(id INTEGER PRIMARY KEY CHECK(id=1), version INTEGER NOT NULL CHECK(version=1));
             CREATE TABLE IF NOT EXISTS task_fences (
                 account_epoch TEXT NOT NULL, task_id TEXT NOT NULL, PRIMARY KEY(account_epoch,task_id));
             CREATE TABLE IF NOT EXISTS local_tasks (
                 account_epoch TEXT NOT NULL, task_id TEXT NOT NULL, owner_epoch TEXT NOT NULL,
                 execution_id TEXT NOT NULL, chat_id TEXT NOT NULL, bot_id TEXT NOT NULL, routine_id TEXT,
                 state TEXT NOT NULL CHECK(state IN ('queued','running','finished','interrupted','needs_review')),
                 cancel_requested INTEGER NOT NULL DEFAULT 0, resolved_at INTEGER, check_report TEXT, PRIMARY KEY(account_epoch,task_id));
             CREATE TABLE IF NOT EXISTS task_invocations (
                 account_epoch TEXT NOT NULL, task_id TEXT NOT NULL, owner_epoch TEXT NOT NULL,
                 execution_id TEXT NOT NULL, invocation_id TEXT NOT NULL, parent_id TEXT,
                 ordinal INTEGER NOT NULL, attempt_id TEXT NOT NULL, receipt_id TEXT NOT NULL,
                 revision INTEGER NOT NULL, digest BLOB NOT NULL CHECK(length(digest)=32),
                 state TEXT NOT NULL CHECK(state IN ('pending','authorized','denied','dismissed','expired','admitted')),
                 kind TEXT, card_id TEXT, answering_device TEXT, answered_at INTEGER,
                 PRIMARY KEY(account_epoch,task_id,attempt_id), UNIQUE(account_epoch,receipt_id),
                 UNIQUE(account_epoch,task_id,execution_id,invocation_id));
             CREATE UNIQUE INDEX IF NOT EXISTS task_invocation_ordinal ON task_invocations(account_epoch,task_id,execution_id,COALESCE(parent_id,''),ordinal);
             CREATE TABLE IF NOT EXISTS task_effect_receipts (
                 account_epoch TEXT NOT NULL, task_id TEXT NOT NULL, owner_epoch TEXT NOT NULL,
                 execution_id TEXT NOT NULL, attempt_id TEXT NOT NULL, receipt_id TEXT NOT NULL,
                 state TEXT NOT NULL CHECK(state IN ('started','finished','failed','unknown')), PRIMARY KEY(account_epoch,receipt_id));
             CREATE TABLE IF NOT EXISTS routine_check_commits (
                 account_epoch TEXT NOT NULL, routine_id TEXT NOT NULL, checked_at INTEGER NOT NULL,
                 PRIMARY KEY(account_epoch,routine_id));
             CREATE TABLE IF NOT EXISTS routine_schedule_authority (
                 account_epoch TEXT NOT NULL, routine_id TEXT NOT NULL, revision TEXT NOT NULL,
                 binding TEXT NOT NULL, next_at INTEGER NOT NULL, revoked INTEGER NOT NULL CHECK(revoked IN (0,1)), PRIMARY KEY(account_epoch,routine_id));
             CREATE TABLE IF NOT EXISTS routine_schedule_occurrences (
                 account_epoch TEXT NOT NULL, routine_id TEXT NOT NULL, revision TEXT NOT NULL,
                 occurrence INTEGER NOT NULL, PRIMARY KEY(account_epoch,routine_id,revision,occurrence));
             ";

    pub fn load_state(&self) -> anyhow::Result<State> {
        let connection = self.connection.lock().unwrap();
        let metadata: Option<(String, i64, i64, Option<String>, bool, bool, String)> = connection
            .query_row(
                "SELECT auto_review_json, last_seq, roster_slot_seq, machine_blob_hash, credentials_uploaded, paused, policy_json
                 FROM metadata WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
            )
            .optional()?;
        let (auto_review, last_seq, roster_slot_seq, machine_blob_hash, credentials_uploaded, paused, policy_json) = match metadata {
            Some((json, last_seq, roster_slot_seq, machine_blob_hash, credentials_uploaded, paused, policy_json)) => (
                serde_json::from_str(&json).context("decoding Auto-review state")?,
                last_seq,
                roster_slot_seq,
                machine_blob_hash,
                credentials_uploaded,
                paused,
                policy_json,
            ),
            None => (Default::default(), 0, 0, None, false, false, "{}".into()),
        };
        let policy: crate::model::RosterBlob = serde_json::from_str(&policy_json).context("decoding policy state")?;
        Ok(State {
            devices: load_json_table(&connection, "devices")?,
            bots: load_json_table(&connection, "bots")?,
            chats: load_json_table(&connection, "chats")?,
            routines: load_json_table(&connection, "routines")?,
            auto_review,
            paused,
            policy_clock: policy.policy_clock,
            pause_version: policy.pause_version,
            capability_versions: policy.capability_versions,
            policy_capabilities: policy.policy_capabilities,
            deleted_bot_versions: policy.deleted_bot_versions,
            last_seq,
            roster_slot_seq,
            group_deletes: load_ordered_ids(&connection, "group_deletes")?,
            blob_deletes: load_ordered_ids(&connection, "blob_deletes")?,
            machine_blob_hash,
            credentials_uploaded,
            device_seen: load_device_seen(&connection)?,
            device_online: Default::default(),
            turns_online: Default::default(),
            device_turns: load_device_turns(&connection)?,
            applied_blob_ids: load_ordered_ids(&connection, "applied_blobs")?,
            listed_machines: Default::default(),
            unknown_machines: Default::default(),
            caught_up: false,
        })
    }

    pub fn save_state(&self, state: &State) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        save_state_tx(&tx, state)?;
        tx.commit()?;
        Ok(())
    }

    pub fn save_state_creating_chat(&self, state: &State, chat: &crate::model::ChatMeta) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        tx.execute("INSERT INTO pending_chat_creates (id, original_json) VALUES (?1, ?2)",
            params![chat.id, serde_json::to_string(chat)?])?;
        save_state_tx(&tx, state)?;
        tx.commit()?;
        Ok(())
    }

    pub fn mark_chat_create_pending(&self, chat: &crate::model::ChatMeta) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute("INSERT OR IGNORE INTO pending_chat_creates (id, original_json) VALUES (?1, ?2)",
            params![chat.id, serde_json::to_string(chat)?])?;
        Ok(())
    }

    pub fn pending_chat_identities(&self) -> anyhow::Result<Vec<(String, Option<crate::model::ChatMeta>)>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare("SELECT id, original_json FROM pending_chat_creates")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))?;
        let identities = rows.map(|result| {
            let (id, json) = result?;
            Ok((id, json.map(|json| serde_json::from_str(&json)).transpose()?))
        }).collect();
        identities
    }

    pub fn acknowledge_chat_creates(&self, ids: &[String]) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        for id in ids { tx.execute("DELETE FROM pending_chat_creates WHERE id = ?1", [id])?; }
        tx.commit()?;
        Ok(())
    }

    #[cfg(test)]
    pub fn forget_chat_create_identity(&self, id: &str) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute("UPDATE pending_chat_creates SET original_json = NULL WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn pending_chat_creates(&self) -> anyhow::Result<Vec<String>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare("SELECT id FROM pending_chat_creates")?;
        let ids = statement.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    pub fn save_state_deleting_chats(
        &self,
        state: &State,
        chat_ids: &[String],
    ) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        save_state_tx(&tx, state)?;
        for chat_id in chat_ids {
            // Fences were inserted at original admission and survive removal of detail.
            tx.execute("DELETE FROM task_effect_receipts WHERE EXISTS(SELECT 1 FROM local_tasks t WHERE t.account_epoch=task_effect_receipts.account_epoch AND t.task_id=task_effect_receipts.task_id AND t.chat_id=?1)", [chat_id])?;
            tx.execute("DELETE FROM task_invocations WHERE EXISTS(SELECT 1 FROM local_tasks t WHERE t.account_epoch=task_invocations.account_epoch AND t.task_id=task_invocations.task_id AND t.chat_id=?1)", [chat_id])?;
            tx.execute("DELETE FROM local_tasks WHERE chat_id=?1", [chat_id])?;
            tx.execute("DELETE FROM messages WHERE chat_id = ?1", [chat_id])?;
            tx.execute("DELETE FROM chat_history WHERE chat_id = ?1", [chat_id])?;
            tx.execute("DELETE FROM codemode_store WHERE chat_id = ?1", [chat_id])?;
            tx.execute(
                "DELETE FROM outbox WHERE group_name = ?1",
                [crate::model::relay_name(chat_id)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn upsert(&self, message: &Message) -> anyhow::Result<Upsert> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        let result = upsert_message_tx(&tx, message)?;
        tx.commit()?;
        Ok(result)
    }

    /// Where this Device's copy of the chat begins in the relay's log, when the relay has
    /// older messages than the ones here. `None` for a chat that is here whole.
    pub fn history_before(&self, chat_id: &str) -> anyhow::Result<Option<i64>> {
        let connection = self.connection.lock().unwrap();
        connection
            .query_row("SELECT before_place FROM chat_history WHERE chat_id = ?1", [chat_id], |row| row.get(0))
            .optional()
            .map_err(Into::into)
    }

    /// `Some(place)`: the relay has messages placed below it that are not here. `None`: the
    /// chat is here whole.
    pub fn set_history_before(&self, chat_id: &str, before: Option<i64>) -> anyhow::Result<()> {
        let connection = self.connection.lock().unwrap();
        match before {
            Some(place) => connection.execute(
                "INSERT INTO chat_history (chat_id, before_place) VALUES (?1, ?2)
                 ON CONFLICT(chat_id) DO UPDATE SET before_place = excluded.before_place",
                params![chat_id, place],
            )?,
            None => connection.execute("DELETE FROM chat_history WHERE chat_id = ?1", [chat_id])?,
        };
        Ok(())
    }

    /// Puts messages read backwards from the relay ahead of everything the chat has here.
    /// `newest_first` is the page in that order, so each lands before the one after it. One
    /// that is here already (a late edit arrived through the log and went to the end) moves
    /// to its place.
    pub fn insert_older(&self, newest_first: &[Message]) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        for message in newest_first {
            let position: i64 = tx.query_row(
                "SELECT COALESCE(MIN(position), 1) - 1 FROM messages WHERE chat_id = ?1",
                [&message.chat_id],
                |row| row.get(0),
            )?;
            let (author_kind, author_bot_id) = author_columns(&message.author);
            let text_nonempty = matches!(&message.body, Body::Text { text, .. } if !text.trim().is_empty());
            tx.execute(
                "INSERT INTO messages (
                     id, chat_id, position, sort_at, created_at, author_kind, author_bot_id,
                     body_kind, is_complete, text_nonempty, message_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id) DO UPDATE SET chat_id = excluded.chat_id, position = excluded.position",
                params![
                    message.id,
                    message.chat_id,
                    position,
                    message.promoted_at.unwrap_or(message.created_at),
                    message.created_at,
                    author_kind,
                    author_bot_id,
                    body_kind(&message.body),
                    message.is_complete(),
                    text_nonempty,
                    serde_json::to_string(message)?,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn message(&self, chat_id: &str, message_id: &str) -> anyhow::Result<Option<Message>> {
        let connection = self.connection.lock().unwrap();
        let json: Option<String> = connection
            .query_row(
                "SELECT message_json FROM messages WHERE chat_id = ?1 AND id = ?2",
                params![chat_id, message_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).context("decoding stored message"))
            .transpose()
    }

    /// Every message of a chat in its order.
    pub fn all(&self, chat_id: &str) -> anyhow::Result<Vec<Message>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection
            .prepare("SELECT message_json FROM messages WHERE chat_id = ?1 ORDER BY position")?;
        let rows = statement.query_map([chat_id], |row| row.get::<_, String>(0))?;
        collect_messages(rows)
    }

    /// The model-visible ordering, with a valid compaction cursor taking precedence over the
    /// fallback limit. Without a valid cursor only the newest `limit` rows are materialized.
    pub fn context(
        &self,
        chat_id: &str,
        after: Option<&str>,
        limit: Option<usize>,
    ) -> anyhow::Result<(Vec<Message>, bool)> {
        let connection = self.connection.lock().unwrap();
        let cursor: Option<(f64, i64)> = match after {
            Some(id) => connection
                .query_row(
                    "SELECT sort_at, position FROM messages WHERE chat_id = ?1 AND id = ?2",
                    params![chat_id, id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?,
            None => None,
        };
        let (mut messages, cursor_found) = if let Some((sort_at, position)) = cursor {
            let mut statement = connection.prepare(
                "SELECT message_json FROM messages
                 WHERE chat_id = ?1 AND (sort_at > ?2 OR (sort_at = ?2 AND position > ?3))
                 ORDER BY sort_at, position",
            )?;
            let rows = statement.query_map(params![chat_id, sort_at, position], |row| {
                row.get::<_, String>(0)
            })?;
            let messages = collect_messages(rows)?;
            (messages, true)
        } else if let Some(limit) = limit {
            let mut statement = connection.prepare(
                "SELECT message_json FROM (
                     SELECT message_json, sort_at, position FROM messages
                     WHERE chat_id = ?1 ORDER BY sort_at DESC, position DESC LIMIT ?2
                 ) ORDER BY sort_at, position",
            )?;
            let rows = statement.query_map(params![chat_id, limit as i64], |row| {
                row.get::<_, String>(0)
            })?;
            let messages = collect_messages(rows)?;
            (messages, false)
        } else {
            let mut statement = connection.prepare(
                "SELECT message_json FROM messages WHERE chat_id = ?1 ORDER BY sort_at, position",
            )?;
            let rows = statement.query_map([chat_id], |row| row.get::<_, String>(0))?;
            let messages = collect_messages(rows)?;
            (messages, false)
        };
        // Defend against an old malformed row without changing the ordering of healthy rows.
        messages.retain(|message| message.chat_id == chat_id);
        Ok((messages, cursor_found))
    }

    /// A page in insertion order, oldest first. `before` is exclusive; an unknown cursor means
    /// the end of the chat, matching the previous in-memory API.
    pub fn page(
        &self,
        chat_id: &str,
        before: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<(Vec<Message>, bool)> {
        let connection = self.connection.lock().unwrap();
        let end = match before {
            Some(id) => connection
                .query_row(
                    "SELECT position FROM messages WHERE chat_id = ?1 AND id = ?2",
                    params![chat_id, id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .unwrap_or(i64::MAX),
            None => i64::MAX,
        };
        let mut statement = connection.prepare(
            "SELECT message_json FROM messages
             WHERE chat_id = ?1 AND position < ?2
             ORDER BY position DESC LIMIT ?3",
        )?;
        let mut messages = collect_messages(
            statement.query_map(params![chat_id, end, (limit + 1) as i64], |row| {
                row.get::<_, String>(0)
            })?,
        )?;
        let has_more = messages.len() > limit;
        if has_more {
            messages.pop();
        }
        messages.reverse();
        Ok((messages, has_more))
    }

    /// The newest `limit` messages after `message_id`, oldest first, and how many of the ones
    /// between it and them were left out.
    pub fn newest_after(&self, chat_id: &str, message_id: &str, limit: usize) -> anyhow::Result<(Vec<Message>, usize)> {
        let connection = self.connection.lock().unwrap();
        let Some(position) = connection
            .query_row("SELECT position FROM messages WHERE chat_id = ?1 AND id = ?2", params![chat_id, message_id], |row| row.get::<_, i64>(0))
            .optional()?
        else {
            return Ok((Vec::new(), 0));
        };
        let total: i64 =
            connection.query_row("SELECT COUNT(*) FROM messages WHERE chat_id = ?1 AND position > ?2", params![chat_id, position], |row| row.get(0))?;
        let mut statement =
            connection.prepare("SELECT message_json FROM messages WHERE chat_id = ?1 AND position > ?2 ORDER BY position DESC LIMIT ?3")?;
        let mut messages = collect_messages(statement.query_map(params![chat_id, position, limit as i64], |row| row.get::<_, String>(0))?)?;
        messages.reverse();
        let left_out = (total as usize).saturating_sub(messages.len());
        Ok((messages, left_out))
    }

    pub fn messages_after(&self, chat_id: &str, message_id: &str) -> anyhow::Result<Vec<Message>> {
        let connection = self.connection.lock().unwrap();
        let Some(position) = connection
            .query_row(
                "SELECT position FROM messages WHERE chat_id = ?1 AND id = ?2",
                params![chat_id, message_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        else {
            return Ok(Vec::new());
        };
        let mut statement = connection.prepare(
            "SELECT message_json FROM messages WHERE chat_id = ?1 AND position > ?2 ORDER BY position",
        )?;
        let rows =
            statement.query_map(params![chat_id, position], |row| row.get::<_, String>(0))?;
        collect_messages(rows)
    }

    /// Every `bash` card (`Body::Tool.run`), in any chat: for the ones a Beans that quit left
    /// open.
    pub fn command_rows(&self) -> anyhow::Result<Vec<Message>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare("SELECT message_json FROM messages WHERE body_kind = 'tool' AND message_json LIKE '%\"run\":{%'")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        collect_messages(rows)
    }

    /// Proposal cards persisted across Runner restarts; the caller checks their decision and waiter.
    pub fn proposal_cards(&self) -> anyhow::Result<Vec<Message>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT message_json FROM messages WHERE body_kind = 'permission' AND message_json LIKE '%\"tool\":\"propose\"%'",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        Ok(collect_messages(rows)?.into_iter().filter(|message| matches!(&message.body, Body::Permission { tool, .. } if tool == "propose")).collect())
    }

    /// A plugin's sign-in cards (`Body::Permission` with `tool` `connect`) in one chat, or in
    /// every chat, in order, each with whether it came after the user last wrote in its chat.
    pub fn sign_in_cards(&self, chat_id: Option<&str>, plugin_id: &str) -> anyhow::Result<Vec<(Message, bool)>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT m.message_json, m.position > COALESCE(
                 (SELECT u.position FROM messages u WHERE u.chat_id = m.chat_id AND u.author_kind = 'you' ORDER BY u.position DESC LIMIT 1),
                 m.position - 1)
             FROM messages m
             WHERE (?1 IS NULL OR m.chat_id = ?1) AND m.body_kind = 'permission' AND m.message_json LIKE '%\"tool\":\"connect\"%'
             ORDER BY m.chat_id, m.position",
        )?;
        let rows = statement.query_map([chat_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)))?;
        let mut cards = Vec::new();
        for row in rows {
            let (json, after_user) = row?;
            let message: Message = serde_json::from_str(&json).context("decoding stored message")?;
            if matches!(&message.body, Body::Permission { plugin_id: p, tool, .. } if p == plugin_id && tool == "connect") {
                cards.push((message, after_user));
            }
        }
        Ok(cards)
    }

    pub fn count_after(&self, chat_id: &str, message_id: Option<&str>) -> anyhow::Result<usize> {
        let connection = self.connection.lock().unwrap();
        let count = match message_id {
            Some(id) => {
                let cursor: Option<(f64, i64)> = connection
                    .query_row(
                        "SELECT sort_at, position FROM messages WHERE chat_id = ?1 AND id = ?2",
                        params![chat_id, id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                match cursor {
                    Some((sort_at, position)) => connection.query_row(
                        "SELECT COUNT(*) FROM messages
                         WHERE chat_id = ?1 AND (sort_at > ?2 OR (sort_at = ?2 AND position > ?3))",
                        params![chat_id, sort_at, position],
                        |row| row.get::<_, i64>(0),
                    )?,
                    None => connection.query_row(
                        "SELECT COUNT(*) FROM messages WHERE chat_id = ?1",
                        [chat_id],
                        |row| row.get::<_, i64>(0),
                    )?,
                }
            }
            None => connection.query_row(
                "SELECT COUNT(*) FROM messages WHERE chat_id = ?1",
                [chat_id],
                |row| row.get::<_, i64>(0),
            )?,
        };
        Ok(count as usize)
    }

    pub fn last_at_or_before(
        &self,
        chat_id: &str,
        timestamp_ms: u64,
    ) -> anyhow::Result<Option<String>> {
        let connection = self.connection.lock().unwrap();
        connection
            .query_row(
                "SELECT id FROM messages WHERE chat_id = ?1 AND sort_at * 1000.0 <= ?2
                 ORDER BY sort_at DESC, position DESC LIMIT 1",
                params![chat_id, timestamp_ms as f64],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn last_user_at(&self) -> anyhow::Result<Option<i64>> {
        let connection = self.connection.lock().unwrap();
        connection
            .query_row(
                "SELECT CAST(MAX(created_at) AS INTEGER) FROM messages WHERE author_kind = 'you'",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    /// What the turn `message_id` started acts on: that message, or the closest one before it
    /// that asks for work: the user's message, a teammate's handoff, or a routine's marker. A
    /// turn that a command's end started opens on the command's card, after the request it served.
    pub fn request_at(&self, chat_id: &str, message_id: &str) -> anyhow::Result<Option<Message>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT message_json FROM messages
             WHERE chat_id = ?1
               AND position <= (SELECT position FROM messages WHERE chat_id = ?1 AND id = ?2)
               AND ((author_kind = 'you' AND body_kind = 'text')
                    OR body_kind = 'handoff'
                    OR (author_kind = 'system' AND body_kind = 'notice'))
             ORDER BY position DESC",
        )?;
        let rows = statement.query_map(params![chat_id, message_id], |row| row.get::<_, String>(0))?;
        for json in rows {
            let message: Message = serde_json::from_str(&json?).context("decoding stored message")?;
            // Other notices only say what happened.
            if !matches!(message.body, Body::Notice { routine_id: None, .. }) {
                return Ok(Some(message));
            }
        }
        Ok(None)
    }

    /// The user's latest text in the chat, from `since` on.
    pub fn last_user_text(&self, chat_id: &str, since: &str) -> anyhow::Result<Option<String>> {
        let connection = self.connection.lock().unwrap();
        let json: Option<String> = connection
            .query_row(
                "SELECT message_json FROM messages
                 WHERE chat_id = ?1 AND author_kind = 'you' AND body_kind = 'text' AND text_nonempty = 1
                   AND position >= (SELECT position FROM messages WHERE chat_id = ?1 AND id = ?2)
                 ORDER BY position DESC LIMIT 1",
                params![chat_id, since],
                |row| row.get(0),
            )
            .optional()?;
        Ok(json
            .map(|json| serde_json::from_str::<Message>(&json).context("decoding stored message"))
            .transpose()?
            .and_then(|message| match message.body {
                Body::Text { text, .. } if !text.trim().is_empty() => Some(text.trim().to_string()),
                _ => None,
            }))
    }

    pub fn last_bot_text(&self, chat_id: &str, bot_id: &str) -> anyhow::Result<Option<Message>> {
        let connection = self.connection.lock().unwrap();
        let json: Option<String> = connection
            .query_row(
                "SELECT message_json FROM messages
                 WHERE chat_id = ?1 AND author_kind = 'bot' AND author_bot_id = ?2
                   AND body_kind = 'text' AND is_complete = 1
                 ORDER BY position DESC LIMIT 1",
                params![chat_id, bot_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).context("decoding stored message"))
            .transpose()
    }

    pub fn text_messages(
        &self,
        chat_id: &str,
        since: Option<i64>,
        until: Option<i64>,
    ) -> anyhow::Result<Vec<Message>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT message_json FROM messages
             WHERE chat_id = ?1 AND body_kind = 'text' AND is_complete = 1
               AND (?2 IS NULL OR created_at >= ?2)
               AND (?3 IS NULL OR created_at <= ?3)
             ORDER BY position",
        )?;
        let rows = statement.query_map(params![chat_id, since, until], |row| {
            row.get::<_, String>(0)
        })?;
        collect_messages(rows)
    }

    pub fn search_messages(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<MessageSearchHit>> {
        let terms = search_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT message_json FROM messages
             WHERE body_kind IN ('text', 'handoff', 'notice', 'permission')
             ORDER BY created_at DESC, position DESC",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut hits = Vec::new();
        for row in rows {
            let message: Message = serde_json::from_str(&row?).context("decoding search result")?;
            let Some(text) = message_search_text(&message) else {
                continue;
            };
            if !search_matches(&text, &terms) {
                continue;
            }
            hits.push(MessageSearchHit {
                message_id: message.id,
                chat_id: message.chat_id,
                snippet: search_snippet(&text, &terms),
                author: message.author,
                created_at: message.created_at,
            });
            if hits.len() >= limit {
                break;
            }
        }
        Ok(hits)
    }

    pub fn heard_count(&self, chat_id: &str, bot_id: &str) -> anyhow::Result<usize> {
        let connection = self.connection.lock().unwrap();
        let count = connection.query_row(
            "SELECT COUNT(*) FROM messages
             WHERE chat_id = ?1 AND is_complete = 1 AND body_kind IN ('text', 'handoff')
               AND NOT (author_kind = 'bot' AND author_bot_id = ?2)",
            params![chat_id, bot_id],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count as usize)
    }

    pub fn new_count_since_bot_spoke(&self, chat_id: &str, bot_id: &str) -> anyhow::Result<usize> {
        let connection = self.connection.lock().unwrap();
        let last: Option<i64> = connection
            .query_row(
                "SELECT position FROM messages
                 WHERE chat_id = ?1 AND author_kind = 'bot' AND author_bot_id = ?2 AND body_kind = 'text'
                 ORDER BY position DESC LIMIT 1",
                params![chat_id, bot_id],
                |row| row.get(0),
            )
            .optional()?;
        let count = connection.query_row(
            "SELECT COUNT(*) FROM messages
             WHERE chat_id = ?1 AND position > ?2 AND is_complete = 1
               AND body_kind IN ('text', 'handoff')",
            params![chat_id, last.unwrap_or(0)],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count as usize)
    }

    /// The values a bot's codemode scripts stored in a chat with `store()`.
    pub fn codemode_values(&self, chat_id: &str, bot_id: &str) -> anyhow::Result<std::collections::BTreeMap<String, serde_json::Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare("SELECT key, json FROM codemode_store WHERE chat_id = ?1 AND bot_id = ?2")?;
        let rows = statement.query_map(params![chat_id, bot_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        let mut values = std::collections::BTreeMap::new();
        for row in rows {
            let (key, json) = row?;
            if let Ok(value) = serde_json::from_str(&json) {
                values.insert(key, value);
            }
        }
        Ok(values)
    }

    /// What one successful script stored and deleted.
    pub fn save_codemode_writes(&self, chat_id: &str, bot_id: &str, set: &std::collections::BTreeMap<String, serde_json::Value>, delete: &[String]) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        for key in delete {
            tx.execute("DELETE FROM codemode_store WHERE chat_id = ?1 AND bot_id = ?2 AND key = ?3", params![chat_id, bot_id, key])?;
        }
        for (key, value) in set {
            tx.execute(
                "INSERT INTO codemode_store (chat_id, bot_id, key, json) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(chat_id, bot_id, key) DO UPDATE SET json = excluded.json",
                params![chat_id, bot_id, key, serde_json::to_string(value)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// A deleted bot's stored script values, in the chats that outlive it.
    pub fn forget_codemode_values_of(&self, bot_id: &str) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute("DELETE FROM codemode_store WHERE bot_id = ?1", [bot_id])?;
        Ok(())
    }

    /// Keeps the stored script values of these bots only: the roster's, once a synced roster
    /// arrives.
    pub fn retain_codemode_bots(&self, bot_ids: &[String]) -> anyhow::Result<()> {
        let valid: std::collections::HashSet<&str> = bot_ids.iter().map(String::as_str).collect();
        let connection = self.connection.lock().unwrap();
        let stored: Vec<String> = {
            let mut statement = connection.prepare("SELECT DISTINCT bot_id FROM codemode_store")?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for bot_id in stored.iter().filter(|bot_id| !valid.contains(bot_id.as_str())) {
            connection.execute("DELETE FROM codemode_store WHERE bot_id = ?1", [bot_id])?;
        }
        Ok(())
    }

    pub fn remove(&self, chat_id: &str, message_id: &str) -> anyhow::Result<bool> {
        let connection = self.connection.lock().unwrap();
        let removed = connection.execute(
            "DELETE FROM messages WHERE chat_id = ?1 AND id = ?2",
            params![chat_id, message_id],
        )? > 0;
        Ok(removed)
    }

    /// Remove rows whose chat metadata no longer exists. The chat table is authoritative;
    /// this catches interrupted or concurrent cleanup before the app serves a snapshot.
    pub fn retain_chats(&self, chat_ids: &[String]) -> anyhow::Result<()> {
        let valid: std::collections::HashSet<&str> = chat_ids.iter().map(String::as_str).collect();
        let valid_groups: std::collections::HashSet<String> = chat_ids
            .iter()
            .map(|id| crate::model::relay_name(id))
            .collect();
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        let stored_chats: Vec<String> = {
            let mut statement = tx.prepare("SELECT DISTINCT chat_id FROM messages")?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for chat_id in stored_chats {
            if !valid.contains(chat_id.as_str()) {
                tx.execute("DELETE FROM messages WHERE chat_id = ?1", [&chat_id])?;
            }
        }
        let scripted_chats: Vec<String> = {
            let mut statement = tx.prepare("SELECT DISTINCT chat_id FROM codemode_store")?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for chat_id in scripted_chats {
            if !valid.contains(chat_id.as_str()) {
                tx.execute("DELETE FROM codemode_store WHERE chat_id = ?1", [&chat_id])?;
            }
        }
        let queued_groups: Vec<String> = {
            let mut statement =
                tx.prepare("SELECT DISTINCT group_name FROM outbox WHERE group_name IS NOT NULL")?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for group in queued_groups {
            if !valid_groups.contains(&group) {
                tx.execute("DELETE FROM outbox WHERE group_name = ?1", [&group])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Queue a relay blob. A newer version of a slot replaces the waiting one without moving
    /// its queue position, so reconnecting still uploads messages in transcript order.
    #[cfg(test)]
    pub fn queue_outbox(&self, item: &OutboxItem) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        queue_outbox_tx(&tx, item)?;
        tx.commit()?;
        Ok(())
    }

    #[cfg(test)]
    pub fn forget_queued_roster_base(&self) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute("DELETE FROM roster_baseline WHERE name = 'queued'", [])?;
        Ok(())
    }

    pub fn roster_baseline(&self, name: &str) -> anyhow::Result<Option<(i64, crate::model::RosterBlob)>> {
        let connection = self.connection.lock().unwrap();
        let row: Option<(i64, String)> = connection.query_row(
            "SELECT seq, json FROM roster_baseline WHERE name = ?1", [name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        row.map(|(seq, json)| Ok((seq, serde_json::from_str(&json)?))).transpose()
    }

    pub fn observe_roster(&self, seq: i64, roster: &crate::model::RosterBlob) -> anyhow::Result<()> {
        self.save_roster_baseline("observed", seq, roster)
    }

    /// Keep the exact snapshot sent before awaiting its response. The outbox may
    /// already contain a newer local edit when a lost response is recovered.
    pub fn record_submitted_roster(&self, seq: i64, roster: &crate::model::RosterBlob) -> anyhow::Result<()> {
        self.save_roster_baseline("submitted", seq, roster)
    }

    fn save_roster_baseline(&self, name: &str, seq: i64, roster: &crate::model::RosterBlob) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute(
            "INSERT INTO roster_baseline (name, seq, json) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET seq = excluded.seq, json = excluded.json
             WHERE excluded.seq >= roster_baseline.seq",
            params![name, seq, serde_json::to_string(roster)?],
        )?;
        Ok(())
    }

    pub fn advance_queued_roster_base(&self) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute(
            "INSERT OR REPLACE INTO roster_baseline (name, seq, json)
             SELECT 'queued', seq, json FROM roster_baseline WHERE name = 'observed'", [],
        )?;
        Ok(())
    }

    pub fn queue_outbox_with_state(&self, item: &OutboxItem, state: &State) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        if item.kind == "roster" {
            tx.execute(
                "INSERT OR IGNORE INTO roster_baseline (name, seq, json)
                 SELECT 'queued', seq, json FROM roster_baseline WHERE name = 'observed'", [],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO roster_baseline (name, seq, json) VALUES ('queued', 0, ?1)",
                [serde_json::to_string(&crate::model::RosterBlob::default())?],
            )?;
        }
        save_metadata_tx(&tx, state)?;
        sync_json_table(
            &tx,
            "devices",
            state
                .devices
                .iter()
                .map(|device| (device.id.clone(), serde_json::to_string(device)))
                .collect::<Vec<_>>(),
        )?;
        sync_chats(&tx, state)?;
        append_applied_blob_tx(&tx, &item.id)?;
        queue_outbox_tx(&tx, item)?;
        tx.commit()?;
        Ok(())
    }
    /// Replace a queued roster only if its snapshot is still current. Keep the baseline and
    /// ciphertext in the same transaction so a local edit cannot be overwritten by a rebase.
    pub fn rebase_queued_roster_with_state(&self, expected_id: &str, item: &OutboxItem, state: &State) -> anyhow::Result<bool> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        let current: Option<String> = tx.query_row(
            "SELECT id FROM outbox WHERE slot_name = 'roster'", [], |row| row.get(0),
        ).optional()?;
        if current.as_deref() != Some(expected_id) { return Ok(false); }
        save_metadata_tx(&tx, state)?;
        append_applied_blob_tx(&tx, &item.id)?;
        queue_outbox_tx(&tx, item)?;
        tx.execute(
            "INSERT OR REPLACE INTO roster_baseline (name, seq, json)
             SELECT 'queued', seq, json FROM roster_baseline WHERE name = 'observed'", [],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Drops what the outbox still holds for a relay group: a chat's messages, read marks,
    /// and attachments.
    pub fn drop_outbox_group(&self, group: &str) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute("DELETE FROM outbox WHERE group_name = ?1", [group])?;
        Ok(())
    }

    pub fn remove_outbox_roster_with_state(&self, id: &str, state: &State, published_chat_ids: &[String]) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        save_metadata_tx(&tx, state)?;
        let removed = tx.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        tx.execute("DELETE FROM roster_baseline WHERE name = 'submitted'", [])?;
        for chat_id in published_chat_ids {
            // A successful PUT publishes these creations even if a newer local edit replaced
            // its outbox slot during the network request.
            tx.execute("DELETE FROM pending_chat_creates WHERE id = ?1", [chat_id])?;
        }
        if removed != 0 {
            tx.execute("DELETE FROM roster_baseline WHERE name = 'queued'", [])?;
        }
        else {
            tx.execute(
                "INSERT OR REPLACE INTO roster_baseline (name, seq, json)
                 SELECT 'queued', seq, json FROM roster_baseline WHERE name = 'observed'", [],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove_outbox_with_state(&self, id: &str, state: &State) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        save_metadata_tx(&tx, state)?;
        tx.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn queued_roster(&self) -> anyhow::Result<Option<OutboxItem>> {
        let connection = self.connection.lock().unwrap();
        connection.query_row(
            "SELECT id, kind, recipient, ciphertext, slot_name, slot_keep_first, group_name FROM outbox WHERE slot_name = 'roster'",
            [], outbox_row,
        ).optional().map_err(Into::into)
    }

    pub fn first_outbox(&self) -> anyhow::Result<Option<OutboxItem>> {
        let connection = self.connection.lock().unwrap();
        connection
            .query_row(
                "SELECT id, kind, recipient, ciphertext, slot_name, slot_keep_first, group_name
                 FROM outbox ORDER BY position LIMIT 1",
                [],
                outbox_row,
            )
            .optional()
            .map_err(Into::into)
    }

    #[cfg(test)]
    pub fn last_outbox(&self) -> anyhow::Result<Option<OutboxItem>> {
        let connection = self.connection.lock().unwrap();
        connection
            .query_row(
                "SELECT id, kind, recipient, ciphertext, slot_name, slot_keep_first, group_name
                 FROM outbox ORDER BY position DESC LIMIT 1",
                [],
                outbox_row,
            )
            .optional()
            .map_err(Into::into)
    }

    /// The turns another Device's latest machine blob lists; none drops its row.
    pub fn set_device_turns(&self, device_id: &str, turns: &[LiveTurn]) -> anyhow::Result<()> {
        let connection = self.connection.lock().unwrap();
        if turns.is_empty() {
            connection.execute("DELETE FROM device_turns WHERE id = ?1", [device_id])?;
        } else {
            connection.execute(
                "INSERT INTO device_turns (id, json) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET json = excluded.json",
                params![device_id, serde_json::to_string(turns)?],
            )?;
        }
        Ok(())
    }

    /// Keeps a job sealed to another Runner until its wait ends.
    pub fn insert_sent_job(&self, job: &SentJob) -> anyhow::Result<()> {
        let connection = self.connection.lock().unwrap();
        connection.execute(
            "INSERT OR REPLACE INTO sent_jobs (id, chat_id, bot_id, routine_id, runner_id, sent_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![job.id, job.chat_id, job.bot_id, job.routine_id, job.runner_id, job.sent_at],
        )?;
        Ok(())
    }

    pub fn remove_sent_job(&self, id: &str) -> anyhow::Result<()> {
        let connection = self.connection.lock().unwrap();
        connection.execute("DELETE FROM sent_jobs WHERE id = ?1", [id])?;
        Ok(())
    }

    /// The jobs sealed to other Runners that this Device still waits on, oldest first.
    pub fn sent_jobs(&self) -> anyhow::Result<Vec<SentJob>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT id, chat_id, bot_id, routine_id, runner_id, sent_at FROM sent_jobs ORDER BY sent_at",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(SentJob {
                id: row.get(0)?,
                chat_id: row.get(1)?,
                bot_id: row.get(2)?,
                routine_id: row.get(3)?,
                runner_id: row.get(4)?,
                sent_at: row.get(5)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    #[cfg(test)]
    pub fn outbox(&self) -> anyhow::Result<Vec<OutboxItem>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT id, kind, recipient, ciphertext, slot_name, slot_keep_first, group_name
             FROM outbox ORDER BY position",
        )?;
        let rows = statement.query_map([], outbox_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn clear(&self) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        for table in [
            "task_authority",
            "task_safety_version",
            "task_fences",
            "task_effect_receipts",
            "task_invocations",
            "local_tasks",
            "routine_check_commits",
            "routine_schedule_authority",
            "routine_schedule_occurrences",
            "memory_config",
            "memory_deliveries",
            "memory_fences",
            "memory_runner_bindings",
            "memory_turn_admissions",
            "roster_baseline",
            "pending_chat_creates",
            "codemode_store",
            "metadata",
            "devices",
            "bots",
            "chats",
            "routines",
            "group_deletes",
            "blob_deletes",
            "device_seen",
            "applied_blobs",
            "messages",
            "chat_history",
            "outbox",
            "sent_jobs",
            "device_turns",
        ] {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.commit()?;
        connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }
}

pub(crate) fn queue_outbox_tx(tx: &Transaction<'_>, item: &OutboxItem) -> anyhow::Result<()> {
    let waiting: Option<i64> = match item.slot.as_ref() {
        Some(slot) => tx
            .query_row(
                "SELECT position FROM outbox WHERE slot_name = ?1",
                [&slot.name],
                |row| row.get(0),
            )
            .optional()?,
        None => None,
    };
    let slot_name = item.slot.as_ref().map(|slot| slot.name.as_str());
    let keep_first = item.slot.as_ref().is_some_and(|slot| slot.keep_first);
    match waiting {
        Some(position) => {
            tx.execute(
                "UPDATE outbox SET id = ?1, kind = ?2, recipient = ?3, ciphertext = ?4,
                     slot_name = ?5, slot_keep_first = ?6, group_name = ?7 WHERE position = ?8",
                params![
                    item.id,
                    item.kind,
                    item.recipient,
                    item.ciphertext,
                    slot_name,
                    keep_first,
                    item.group,
                    position
                ],
            )?;
        }
        None => {
            tx.execute(
                    "INSERT INTO outbox (id, kind, recipient, ciphertext, slot_name, slot_keep_first, group_name)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![item.id, item.kind, item.recipient, item.ciphertext, slot_name, keep_first, item.group],
                )?;
        }
    }
    Ok(())
}

fn reconcile_schedules_tx(tx: &Transaction<'_>, state: &State) -> anyhow::Result<()> {
    let authorities: Vec<(String,String)> = tx.prepare("SELECT routine_id,binding FROM routine_schedule_authority WHERE revoked=0")?
        .query_map([],|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    for (id,binding) in authorities {
        let current=state.routines.iter().find(|r|r.id==id).map(|r|schedule_binding(state,r)).transpose()?;
        if current.as_deref()!=Some(binding.as_str()) {
            tx.execute("UPDATE routine_schedule_authority SET revoked=1 WHERE routine_id=?1",[id])?;
        }
    }
    Ok(())
}

fn save_state_tx(tx: &Transaction<'_>, state: &State) -> anyhow::Result<()> {
    reconcile_schedules_tx(tx,state)?;
    save_metadata_tx(tx, state)?;
    sync_json_table(
        tx,
        "devices",
        state
            .devices
            .iter()
            .map(|device| (device.id.clone(), serde_json::to_string(device)))
            .collect::<Vec<_>>(),
    )?;
    sync_json_table(
        tx,
        "bots",
        state
            .bots
            .iter()
            .map(|bot| (bot.id.clone(), serde_json::to_string(bot)))
            .collect::<Vec<_>>(),
    )?;
    sync_chats(tx, state)?;
    sync_json_table(
        tx,
        "routines",
        state
            .routines
            .iter()
            .map(|routine| (routine.id.clone(), serde_json::to_string(routine)))
            .collect::<Vec<_>>(),
    )?;
    sync_ordered_ids(tx, "group_deletes", &state.group_deletes)?;
    sync_ordered_ids(tx, "blob_deletes", &state.blob_deletes)?;
    sync_ordered_ids(tx, "applied_blobs", &state.applied_blob_ids)?;
    sync_device_seen(tx, &state.device_seen)?;
    Ok(())
}

fn save_metadata_tx(tx: &Transaction<'_>, state: &State) -> anyhow::Result<()> {
    tx.execute(
        "INSERT INTO metadata (id, auto_review_json, last_seq, roster_slot_seq, machine_blob_hash, credentials_uploaded, paused, policy_json)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
             auto_review_json = excluded.auto_review_json,
             last_seq = excluded.last_seq,
             roster_slot_seq = excluded.roster_slot_seq,
             machine_blob_hash = excluded.machine_blob_hash,
             credentials_uploaded = excluded.credentials_uploaded,
             paused = excluded.paused,
             policy_json = excluded.policy_json",
        params![
            serde_json::to_string(&state.auto_review)?,
            state.last_seq,
            state.roster_slot_seq,
            state.machine_blob_hash,
            state.credentials_uploaded,
            state.paused,
            serde_json::to_string(&crate::model::RosterBlob {
                policy_clock: state.policy_clock,
                pause_version: state.pause_version.clone(),
                capability_versions: state.capability_versions.clone(),
                policy_capabilities: state.policy_capabilities.clone(),
                deleted_bot_versions: state.deleted_bot_versions.clone(),
                ..Default::default()
            })?,
        ],
    )?;
    Ok(())
}

fn sync_chats(tx: &Transaction<'_>, state: &State) -> anyhow::Result<()> {
    sync_json_table(
        tx,
        "chats",
        state
            .chats
            .iter()
            .map(|chat| (chat.meta.id.clone(), serde_json::to_string(chat)))
            .collect::<Vec<_>>(),
    )
}

fn message_search_text(message: &Message) -> Option<String> {
    let text = match &message.body {
        Body::Text { text, attachments, .. } => {
            let names = attachments
                .iter()
                .map(|attachment| attachment.name.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            format!("{text} {names}")
        }
        Body::Handoff { reason, .. } => reason.clone(),
        Body::Notice { text, .. } => text.clone(),
        Body::Permission { summary, .. } => summary.clone(),
        Body::Tool { .. } => return None,
    };
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

pub(crate) fn search_terms(input: &str) -> Vec<String> {
    input
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(str::to_lowercase)
        .collect()
}

pub(crate) fn search_matches(text: &str, terms: &[String]) -> bool {
    let text = text.to_lowercase();
    terms.iter().all(|term| text.contains(term))
}

pub(crate) fn search_snippet(text: &str, terms: &[String]) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return String::new();
    }
    let matched = words
        .iter()
        .position(|word| {
            let word = word.to_lowercase();
            terms.iter().any(|term| word.contains(term))
        })
        .unwrap_or(0);
    let start = matched.saturating_sub(8);
    let end = (matched + 16).min(words.len());
    format!(
        "{}{}{}",
        if start > 0 { "… " } else { "" },
        words[start..end].join(" "),
        if end < words.len() { " …" } else { "" }
    )
}

fn append_applied_blob_tx(tx: &Transaction<'_>, id: &str) -> anyhow::Result<()> {
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position), 0) + 1 FROM applied_blobs",
        [],
        |row| row.get(0),
    )?;
    tx.execute(
        "INSERT INTO applied_blobs (id, position) VALUES (?1, ?2)
         ON CONFLICT(id) DO UPDATE SET position = excluded.position",
        params![id, position],
    )?;
    tx.execute(
        "DELETE FROM applied_blobs
         WHERE id NOT IN (SELECT id FROM applied_blobs ORDER BY position DESC LIMIT 2000)",
        [],
    )?;
    Ok(())
}

fn sync_json_table(
    tx: &Transaction<'_>,
    table: &str,
    rows: Vec<(String, serde_json::Result<String>)>,
) -> anyhow::Result<()> {
    let existing: std::collections::HashMap<String, (i64, String)> = {
        let mut statement = tx.prepare(&format!("SELECT id, position, json FROM {table}"))?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut retained = std::collections::HashSet::new();
    let upsert = format!(
        "INSERT INTO {table} (id, position, json) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET position = excluded.position, json = excluded.json"
    );
    for (position, (id, json)) in rows.into_iter().enumerate() {
        let json = json?;
        let position = position as i64;
        if existing
            .get(&id)
            .is_none_or(|stored| stored.0 != position || stored.1 != json)
        {
            tx.execute(&upsert, params![id, position, json])?;
        }
        retained.insert(id);
    }
    delete_missing_from(tx, table, existing.keys(), &retained)
}

fn sync_ordered_ids(tx: &Transaction<'_>, table: &str, rows: &[String]) -> anyhow::Result<()> {
    let existing: Vec<(String, i64)> = {
        let mut statement = tx.prepare(&format!(
            "SELECT id, position FROM {table} ORDER BY position"
        ))?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let existing_ids: std::collections::HashSet<&str> =
        existing.iter().map(|(id, _)| id.as_str()).collect();
    let retained: std::collections::HashSet<String> = rows.iter().cloned().collect();
    delete_missing_from(tx, table, existing.iter().map(|(id, _)| id), &retained)?;
    let mut position = existing
        .iter()
        .map(|(_, position)| *position)
        .max()
        .unwrap_or(0);
    let insert = format!("INSERT INTO {table} (id, position) VALUES (?1, ?2)");
    for id in rows {
        if !existing_ids.contains(id.as_str()) {
            position += 1;
            tx.execute(&insert, params![id, position])?;
        }
    }
    Ok(())
}

fn sync_device_seen(
    tx: &Transaction<'_>,
    rows: &std::collections::HashMap<String, i64>,
) -> anyhow::Result<()> {
    let existing: std::collections::HashMap<String, i64> = {
        let mut statement = tx.prepare("SELECT id, seen_at FROM device_seen")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut retained = std::collections::HashSet::new();
    for (id, seen_at) in rows {
        if existing.get(id) != Some(seen_at) {
            tx.execute(
                "INSERT INTO device_seen (id, seen_at) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET seen_at = excluded.seen_at",
                params![id, seen_at],
            )?;
        }
        retained.insert(id.clone());
    }
    delete_missing_from(tx, "device_seen", existing.keys(), &retained)
}

fn delete_missing_from<'a>(
    tx: &Transaction<'_>,
    table: &str,
    existing: impl Iterator<Item = &'a String>,
    retained: &std::collections::HashSet<String>,
) -> anyhow::Result<()> {
    let delete = format!("DELETE FROM {table} WHERE id = ?1");
    for id in existing {
        if !retained.contains(id) {
            tx.execute(&delete, [id])?;
        }
    }
    Ok(())
}

fn load_json_table<T: DeserializeOwned>(
    connection: &Connection,
    table: &str,
) -> anyhow::Result<Vec<T>> {
    let values: Vec<String> = {
        let mut statement =
            connection.prepare(&format!("SELECT json FROM {table} ORDER BY position"))?;
        let rows = statement.query_map([], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    values
        .into_iter()
        .map(|json| serde_json::from_str(&json).with_context(|| format!("decoding {table} row")))
        .collect()
}

fn load_ordered_ids(connection: &Connection, table: &str) -> anyhow::Result<Vec<String>> {
    let mut statement = connection.prepare(&format!("SELECT id FROM {table} ORDER BY position"))?;
    let rows = statement.query_map([], |row| row.get(0))?;
    rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
}

fn load_device_turns(
    connection: &Connection,
) -> anyhow::Result<std::collections::HashMap<String, Vec<LiveTurn>>> {
    let mut statement = connection.prepare("SELECT id, json FROM device_turns")?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    rows.map(|row| {
        let (id, json) = row?;
        Ok((id, serde_json::from_str(&json).context("decoding a Device's turns")?))
    })
    .collect()
}

fn load_device_seen(
    connection: &Connection,
) -> anyhow::Result<std::collections::HashMap<String, i64>> {
    let mut statement = connection.prepare("SELECT id, seen_at FROM device_seen")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
}

fn collect_messages(
    rows: impl Iterator<Item = rusqlite::Result<String>>,
) -> anyhow::Result<Vec<Message>> {
    rows.map(|row| {
        let json = row?;
        serde_json::from_str(&json).context("decoding stored message")
    })
    .collect()
}

fn author_columns(author: &Author) -> (&'static str, Option<&str>) {
    match author {
        Author::You => ("you", None),
        Author::Bot { bot_id } => ("bot", Some(bot_id)),
        Author::System => ("system", None),
    }
}

fn body_kind(body: &Body) -> &'static str {
    match body {
        Body::Text { .. } => "text",
        Body::Tool { .. } => "tool",
        Body::Handoff { .. } => "handoff",
        Body::Notice { .. } => "notice",
        Body::Permission { .. } => "permission",
    }
}

fn outbox_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutboxItem> {
    let slot_name: Option<String> = row.get(4)?;
    let keep_first = row.get::<_, i64>(5)? != 0;
    Ok(OutboxItem {
        id: row.get(0)?,
        kind: row.get(1)?,
        recipient: row.get(2)?,
        ciphertext: row.get(3)?,
        slot: slot_name.map(|name| Slot { name, keep_first }),
        group: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MessageState, SNAPSHOT_MESSAGES};

    struct Scratch(LocalStore, std::path::PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    fn scratch() -> Scratch {
        let home = std::env::temp_dir().join(format!("beans-transcript-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        Scratch(LocalStore::open(&home.join("beans.sqlite3")).unwrap(), home)
    }

    fn task_fixture(store: &LocalStore, id: &str) -> (TaskLease,crate::model::Job) {
        let owner = "owner-test";
        let epoch = store.current_task_account_epoch().unwrap().unwrap_or_else(||store.recover_task_owner(owner).unwrap());
        let lease = TaskLease { account_epoch:epoch,owner_epoch:owner.into(),task_id:id.into(),execution_id:uuid::Uuid::new_v4().to_string(),incarnation:0 };
        let job = crate::model::Job { id:id.into(),chat_id:"chat-test".into(),bot_id:"bot-test".into(),kind:"turn".into(),trigger_message_id:String::new(),routine_id:None,check:None,requested_by:"device-test".into(),from_bot_id:None,hops:0,round:0,is_winding_down:false,setup:None,created_at:0.0 };
        (lease,job)
    }

    fn invocation() -> InvocationBinding {
        InvocationBinding { invocation_id:"invocation-test".into(),parent_invocation_id:None,ordinal:0,attempt_id:"attempt-test".into(),receipt_id:"receipt-test".into(),revision:1,digest:[42;32] }
    }

    #[test]
    fn task_duplicate_cancel_and_prune_keep_replay_denial() {
        let scratch = scratch(); let store = &scratch.0;
        let (lease,job) = task_fixture(store,"job-test");
        assert!(store.queue_task(&lease,&job).unwrap());
        assert!(!store.queue_task(&lease,&job).unwrap());
        store.cancel_tasks(&lease.account_epoch,Some(&job.id),None,None).unwrap();
        assert!(!store.start_task(&lease).unwrap());
        store.recover_task_owner("owner-test").unwrap();
        assert_eq!(store.task_state(&lease).unwrap(),Some(TaskState::Interrupted));
        assert!(store.resolve_task(&lease,0).unwrap());
        store.prune_tasks(30*24*60*60+1).unwrap();
        assert_eq!(store.task_state(&lease).unwrap(),None);
        assert!(!store.queue_task(&lease,&job).unwrap());
        let (cancelled,new_job) = task_fixture(store,"job-cancel-first");
        store.cancel_tasks(&cancelled.account_epoch,Some(&new_job.id),None,None).unwrap();
        assert!(!store.queue_task(&cancelled,&new_job).unwrap());
    }

    #[test]
    fn schema_admission_profiles_corruption_and_atomic_rollback() {
        for (version, profile) in [(1,SchemaProfile::PreTask),(1,SchemaProfile::InitialTask),(1,SchemaProfile::Core),(2,SchemaProfile::Core),(3,SchemaProfile::Scheduled),(4,SchemaProfile::Current)] {
            let mut conn = Connection::open_in_memory().unwrap();
            conn.execute_batch("PRAGMA synchronous=FULL; PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON;").unwrap();
            conn.pragma_update(None,"application_id",crate::config::SQLITE_APPLICATION_ID).unwrap();
            for sql in LocalStore::SCHEMA.split(';').filter(|s| !s.trim().is_empty()) {
                let tokens=schema_tokens(sql).unwrap();
                let name=&tokens[if tokens[1]=="unique" {3} else {2}];
                if task_schema_name(name) && (profile==SchemaProfile::PreTask || (matches!(profile,SchemaProfile::InitialTask|SchemaProfile::Core) && name.starts_with("routine_schedule_")) || (profile==SchemaProfile::InitialTask && name=="task_safety_version")) {continue;}
                let sql=if profile==SchemaProfile::InitialTask && name=="task_authority" {sql.replace(", admission_floor REAL", "")} else {sql.to_string()};
                conn.execute_batch(&sql).unwrap();
            }
            if profile==SchemaProfile::Current {
                let tx=conn.transaction().unwrap(); crate::artifacts::install_proposal_artifact_schema_tx(&tx).unwrap(); tx.commit().unwrap();
            }
            conn.pragma_update(None,"user_version",version).unwrap();
            conn.execute("INSERT INTO messages VALUES('kept','chat',1,0,0,'you',NULL,'text',1,1,'retained')",[]).unwrap();
            assert_eq!(classify_schema(&conn).unwrap(),profile);
            {
                let tx=conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
                install_local_schema_tx(&tx,profile).unwrap();
                assert_eq!(classify_schema(&tx).unwrap(),SchemaProfile::Current);
                // A later sibling failure rolls back task/artifact DDL and the marker.
                tx.rollback().unwrap();
            }
            assert_eq!(classify_schema(&conn).unwrap(),profile);
            let retained:String=conn.query_row("SELECT message_json FROM messages WHERE id='kept'",[],|r|r.get(0)).unwrap();
            assert_eq!(retained,"retained");
            let tx=conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
            install_local_schema_tx(&tx,profile).unwrap(); tx.commit().unwrap();
            assert_eq!(classify_schema(&conn).unwrap(),SchemaProfile::Current);
            conn.execute_batch("ALTER TABLE task_authority ADD COLUMN unexpected TEXT;").unwrap();
            assert!(classify_schema(&conn).is_err());
            conn.execute_batch("ALTER TABLE task_authority DROP COLUMN unexpected;").unwrap();
            conn.execute_batch("DROP INDEX task_invocation_ordinal; CREATE INDEX task_invocation_ordinal ON task_invocations(account_epoch,task_id,execution_id,ordinal);").unwrap();
            assert!(classify_schema(&conn).is_err());
        }
        let conn=Connection::open_in_memory().unwrap();
        assert_eq!(classify_schema(&conn).unwrap(),SchemaProfile::Empty);
        conn.execute_batch("PRAGMA synchronous=FULL; PRAGMA fullfsync=ON;").unwrap();
        {
            let mut conn=Connection::open_in_memory().unwrap();
            conn.execute_batch("PRAGMA synchronous=FULL; PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON;").unwrap();
            let tx=conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
            install_local_schema_tx(&tx,SchemaProfile::Empty).unwrap();
            assert!(tx.execute("INSERT INTO task_safety_version VALUES(2,1)",[]).is_err());
            tx.rollback().unwrap();
            assert_eq!(classify_schema(&conn).unwrap(),SchemaProfile::Empty);
        }
        conn.execute_batch("CREATE TABLE unrelated(value TEXT);").unwrap();
        assert!(classify_schema(&conn).is_err());
        for version in [1,2,3,4,5] {conn.pragma_update(None,"user_version",version).unwrap(); assert!(classify_schema(&conn).is_err());}
        let conn=Connection::open_in_memory().unwrap();
        conn.execute_batch(LocalStore::SCHEMA).unwrap();
        conn.pragma_update(None,"application_id",crate::config::SQLITE_APPLICATION_ID).unwrap();
        for version in [0,2,5] {conn.pragma_update(None,"user_version",version).unwrap(); assert!(classify_schema(&conn).is_err());}
        conn.pragma_update(None,"user_version",3).unwrap();
        conn.execute_batch("PRAGMA writable_schema=ON; UPDATE sqlite_master SET sql=replace(sql,'length(digest)=32','length(digest)=31') WHERE name='task_invocations'; PRAGMA writable_schema=OFF;").unwrap();
        assert!(classify_schema(&conn).is_err());
        conn.execute_batch("PRAGMA writable_schema=ON; UPDATE sqlite_master SET sql=replace(sql,'length(digest)=31','length(digest)=32') WHERE name='task_invocations'; PRAGMA writable_schema=OFF;").unwrap();
        conn.execute_batch("CREATE TRIGGER task_extra AFTER INSERT ON local_tasks BEGIN DELETE FROM task_fences; END;").unwrap();
        assert!(classify_schema(&conn).is_err());
        conn.execute_batch("DROP TRIGGER task_extra; DROP TABLE task_effect_receipts; CREATE VIEW task_effect_receipts AS SELECT * FROM local_tasks;").unwrap();
        assert!(classify_schema(&conn).is_err());
    }

    #[test]
    fn task_recovery_unknown_is_immutable_and_old_owner_is_stale() {
        let scratch = scratch(); let store = &scratch.0;
        let (lease,job) = task_fixture(store,"job-test"); let binding=invocation();
        assert!(store.queue_task(&lease,&job).unwrap()); assert!(store.start_task(&lease).unwrap());
        assert!(store.insert_invocation(&lease,&binding).unwrap());
        assert!(store.decide_invocation(&lease,&binding,AuthorizationDecision::Authorized,AuthorizationKind::Rule,None,None,0).unwrap());
        let rejected: anyhow::Result<()> = store.safety(|tx| {
            assert!(admit_invocation_tx(tx, &lease, &binding)?);
            anyhow::bail!("synthetic sibling mutation rejection")
        });
        assert!(rejected.is_err());
        {
            let connection = store.connection.lock().unwrap();
            let state: String = connection.query_row("SELECT state FROM task_invocations WHERE account_epoch=?1 AND task_id=?2 AND invocation_id=?3", params![lease.account_epoch,lease.task_id,binding.invocation_id], |row| row.get(0)).unwrap();
            assert_eq!(state, "authorized");
            let receipt: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM task_effect_receipts WHERE account_epoch=?1 AND task_id=?2 AND attempt_id=?3 AND receipt_id=?4)", params![lease.account_epoch,lease.task_id,binding.attempt_id,binding.receipt_id], |row| row.get(0)).unwrap();
            assert!(!receipt);
        }
        assert!(store.admit_invocation(&lease,&binding).unwrap());
        assert!(!store.admit_invocation(&lease,&binding).unwrap());
        assert!(store.finish_receipt(&lease,&binding,ReceiptOutcome::Unknown).unwrap());
        assert!(!store.finish_receipt(&lease,&binding,ReceiptOutcome::Finished).unwrap());
        store.recover_task_owner("owner-replacement").unwrap();
        assert_eq!(store.task_state(&lease).unwrap(),Some(TaskState::NeedsReview));
        assert!(store.finish_receipt(&lease,&binding,ReceiptOutcome::Finished).is_err());
        store.close_task_account().unwrap(); store.clear().unwrap();
        assert!(store.finish_receipt(&lease,&binding,ReceiptOutcome::Finished).is_err());
        let count:i64=store.connection.lock().unwrap().query_row("SELECT count(*) FROM task_effect_receipts",[],|r|r.get(0)).unwrap();
        assert_eq!(count,0);
    }

    #[test]
    fn routine_check_and_task_insert_roll_back_together() {
        let scratch = scratch(); let store=&scratch.0;
        let (lease,mut job)=task_fixture(store,"job-check"); job.routine_id=Some("routine-test".into());
        let set=std::collections::BTreeMap::from([("seen".into(),serde_json::json!(true))]);
        store.connection.lock().unwrap().execute_batch("CREATE TRIGGER refuse_check BEFORE INSERT ON routine_check_commits BEGIN SELECT RAISE(ABORT,'synthetic rejection'); END;").unwrap();
        assert!(store.commit_routine_check(&lease,Some(&job),"routine-test",&job.chat_id,&job.bot_id,1,&set,&[]).is_err());
        assert_eq!(store.task_state(&lease).unwrap(),None);
        assert!(!store.codemode_values(&job.chat_id,&job.bot_id).unwrap().contains_key("seen"));
        store.connection.lock().unwrap().execute_batch("DROP TRIGGER refuse_check;").unwrap();
        assert!(store.commit_routine_check(&lease,Some(&job),"routine-test",&job.chat_id,&job.bot_id,1,&set,&[]).unwrap());
        assert_eq!(store.task_state(&lease).unwrap(),Some(TaskState::Queued));
        assert_eq!(store.codemode_values(&job.chat_id,&job.bot_id).unwrap().get("seen"),Some(&serde_json::json!(true)));
    }

    /// The retained binding loader is all-or-nothing: a row that is not a complete structural
    /// binding is an error, never a partially rebuilt one, and freezing an origin twice never
    /// remints an attempt.
    #[test]
    fn frozen_invocation_loads_retained_identity_and_rejects_a_corrupt_row() {
        let scratch=scratch(); let store=&scratch.0;
        let (lease,job)=task_fixture(store,"job-freeze");
        assert!(store.queue_task(&lease,&job).unwrap());
        assert!(store.start_task(&lease).unwrap());
        let frozen=store.freeze_or_load_invocation(&lease,"msg-propose-test",1,[7;32],"attempt-a","receipt-a").unwrap();
        assert!(frozen.created);
        assert_eq!(frozen.state,InvocationState::Pending);
        assert_eq!(frozen.binding.ordinal,0);
        let again=store.freeze_or_load_invocation(&lease,"msg-propose-test",1,[7;32],"attempt-b","receipt-b").unwrap();
        assert!(!again.created);
        assert_eq!(again.binding.attempt_id,"attempt-a");
        assert_eq!(again.binding.receipt_id,"receipt-a");
        assert_eq!(again.binding.ordinal,0);
        let second=store.freeze_or_load_invocation(&lease,"msg-propose-other",1,[8;32],"attempt-c","receipt-c").unwrap();
        assert_eq!(second.binding.ordinal,1);
        let (loaded,state)=store.load_invocation(&lease,"msg-propose-test").unwrap().unwrap();
        assert_eq!(loaded,frozen.binding);
        assert_eq!(state,InvocationState::Pending);
        assert!(store.load_invocation(&lease,"msg-propose-absent").unwrap().is_none());
        store.connection.lock().unwrap().execute("UPDATE task_invocations SET digest=?2 WHERE invocation_id=?1",params!["msg-propose-test",&vec![1u8;16][..]]).unwrap();
        assert!(store.load_invocation(&lease,"msg-propose-test").is_err(),"a short digest must not load as a binding");
        assert!(store.freeze_or_load_invocation(&lease,"msg-propose-test",1,[7;32],"attempt-d","receipt-d").is_err(),"a corrupt retained row must not be silently reminted");
        store.connection.lock().unwrap().execute("UPDATE task_invocations SET digest=?2 WHERE invocation_id=?1",params!["msg-propose-test",&vec![7u8;32][..]]).unwrap();
        store.connection.lock().unwrap().execute("UPDATE task_invocations SET state='not-a-state' WHERE invocation_id='msg-propose-test'", []).unwrap();
        assert!(store.load_invocation(&lease,"msg-propose-test").is_err(),"unknown persisted state must not be guessed");
    }

    #[test]
    fn legacy_owner_migration_preserves_data_and_denies_prior_work() {
        let scratch=scratch(); let store=&scratch.0;
        store.upsert(&message("legacy-message",1.0)).unwrap();
        let item=OutboxItem { id:"legacy-outbox".into(),kind:"job".into(),recipient:Some("remote-runner".into()),ciphertext:vec![1,2,3],slot:None,group:None };
        store.queue_outbox_with_state(&item,&State::default()).unwrap();
        let before_state=store.load_state().unwrap();
        let before_outbox=store.last_outbox().unwrap().unwrap();
        let sent=SentJob { id:"job-legacy".into(),chat_id:"chat-test".into(),bot_id:"bot-test".into(),routine_id:None,runner_id:"runner-test".into(),sent_at:1.0 };
        store.insert_sent_job(&sent).unwrap();
        store.connection.lock().unwrap().execute("INSERT INTO memory_turn_admissions VALUES('job-memory','{}')",[]).unwrap();
        let before=store.message("chat","legacy-message").unwrap();
        let epoch=store.recover_task_owner("owner-test").unwrap();
        assert_eq!(store.message("chat","legacy-message").unwrap(),before);
        assert_eq!(store.load_state().unwrap().last_seq,before_state.last_seq);
        let after_outbox=store.last_outbox().unwrap().unwrap();
        assert_eq!(after_outbox.id,before_outbox.id);
        assert_eq!(after_outbox.ciphertext,before_outbox.ciphertext);
        assert_eq!(store.sent_jobs().unwrap(),vec![sent]);
        let (mut lease,mut job)=task_fixture(store,"job-legacy"); lease.account_epoch=epoch;
        job.created_at=crate::config::now_secs()+1.0;
        assert!(!store.queue_task(&lease,&job).unwrap(),"known legacy identity stays denied even with a new timestamp");
        lease.task_id="job-unseen-old".into(); job.id=lease.task_id.clone(); job.created_at=1.0;
        assert!(!store.queue_task(&lease,&job).unwrap());
        job.created_at=crate::config::now_secs()+1.0;
        assert!(!store.queue_task(&lease,&job).unwrap(),"denied legacy envelope has a lifetime fence");
        lease.task_id="job-explicit-new".into(); job.id=lease.task_id.clone();
        assert!(!store.queue_task(&lease,&job).unwrap(),"legacy-unseen execution stays closed without new-intent proof");
        assert_eq!(store.task_state(&lease).unwrap(),None);
    }
    #[test]
    fn legacy_unseen_future_timestamp_replay_is_denied_across_reopen() {
        let home=tempfile::tempdir().unwrap(); let path=home.path().join("beans.sqlite3");
        let store=LocalStore::open(&path).unwrap();
        store.upsert(&message("legacy-transcript",1.0)).unwrap();
        let epoch=store.recover_task_owner("owner-test").unwrap();
        let (lease,mut job)=task_fixture(&store,"job-previously-executed-unseen");
        job.created_at=4_000_000_000.0;
        assert!(!store.queue_task(&lease,&job).unwrap());
        assert_eq!(store.task_state(&lease).unwrap(),None);
        drop(store);
        let reopened=LocalStore::open(&path).unwrap();
        assert_eq!(reopened.recover_task_owner("owner-test").unwrap(),epoch);
        assert!(!reopened.queue_task(&lease,&job).unwrap());
        assert_eq!(reopened.task_state(&lease).unwrap(),None);
    }

    #[test]
    fn current_schema_missing_all_safety_tables_is_not_a_predecessor() {
        let home=tempfile::tempdir().unwrap(); let path=home.path().join("beans.sqlite3");
        let store=LocalStore::open(&path).unwrap();
        store.recover_task_owner("owner-test").unwrap();
        store.connection.lock().unwrap().execute_batch("DROP TABLE task_authority; DROP TABLE task_fences; DROP TABLE local_tasks; DROP TABLE task_invocations; DROP TABLE task_effect_receipts; DROP TABLE routine_check_commits; DROP TABLE task_safety_version;").unwrap();
        drop(store);
        assert!(LocalStore::open(&path).is_err());
    }

    #[test]
    fn supported_predecessor_keeps_serving_data_without_execution() {
        let home=tempfile::tempdir().unwrap(); let path=home.path().join("beans.sqlite3");
        let store=LocalStore::open(&path).unwrap();
        store.upsert(&message("legacy-transcript",1.0)).unwrap();
        store.connection.lock().unwrap().execute_batch("DROP TABLE task_authority; DROP TABLE task_fences; DROP TABLE local_tasks; DROP TABLE task_invocations; DROP TABLE task_effect_receipts; DROP TABLE routine_check_commits; DROP TABLE task_safety_version; DROP TABLE routine_schedule_authority; DROP TABLE routine_schedule_occurrences; PRAGMA user_version=1;").unwrap();
        drop(store);
        let reopened=LocalStore::open(&path).unwrap();
        reopened.recover_task_owner("owner-test").unwrap();
        assert!(reopened.message("chat","legacy-transcript").unwrap().is_some());
        let (lease,mut job)=task_fixture(&reopened,"job-unseen"); job.created_at=4_000_000_000.0;
        assert!(!reopened.queue_task(&lease,&job).unwrap());
        reopened.close_task_account().unwrap(); reopened.clear().unwrap();
        reopened.task_account_epoch("owner-test").unwrap();
        let marked:bool=reopened.connection.lock().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM task_safety_version)",[],|r|r.get(0)).unwrap();
        assert!(marked);
    }


    #[test]
    fn legacy_migration_failure_leaves_authority_and_journals_unchanged() {
        let scratch=scratch(); let store=&scratch.0;
        store.connection.lock().unwrap().execute("INSERT INTO device_turns VALUES('legacy-device','invalid-json')",[]).unwrap();
        assert!(store.recover_task_owner("owner-test").is_err());
        assert_eq!(store.current_task_account_epoch().unwrap(),None);
        let json:String=store.connection.lock().unwrap().query_row("SELECT json FROM device_turns WHERE id='legacy-device'",[],|r|r.get(0)).unwrap();
        assert_eq!(json,"invalid-json");
        let count:i64=store.connection.lock().unwrap().query_row("SELECT count(*) FROM task_fences",[],|r|r.get(0)).unwrap();
        assert_eq!(count,0);
    }

    #[test]
    fn recovered_routine_resolution_keeps_unknown_and_replay_fence() {
        let scratch=scratch(); let store=&scratch.0;
        let (lease,mut job)=task_fixture(store,"job-resolve"); job.routine_id=Some("routine-test".into());
        assert!(store.queue_task(&lease,&job).unwrap()); assert!(store.start_task(&lease).unwrap());
        let binding=invocation();
        assert!(store.insert_invocation(&lease,&binding).unwrap());
        assert!(store.decide_invocation(&lease,&binding,AuthorizationDecision::Authorized,AuthorizationKind::Rule,None,None,1).unwrap());
        assert!(store.admit_invocation(&lease,&binding).unwrap());
        store.recover_task_owner("owner-new").unwrap();
        assert!(store.routine_needs_review(&lease.account_epoch,"routine-test").unwrap());
        let history=store.history_task_lease("owner-new",&job.id,0).unwrap().unwrap();
        assert!(store.resolve_task(&history,2).unwrap());
        assert!(!store.routine_needs_review(&lease.account_epoch,"routine-test").unwrap());
        assert!(store.finish_receipt(&lease,&binding,ReceiptOutcome::Finished).is_err());
        let state:String=store.connection.lock().unwrap().query_row("SELECT state FROM task_effect_receipts WHERE receipt_id=?1",[&binding.receipt_id],|r|r.get(0)).unwrap();
        assert_eq!(state,"unknown");
    }

    fn message(id: &str, at: f64) -> Message {
        let mut message = Message::new("chat", Author::You, Body::text(id));
        message.id = id.into();
        message.created_at = at;
        message
    }

    #[test]
    fn older_pages_land_before_what_is_here() {
        let scratch = scratch();
        let store = &scratch.0;
        let said = |id: &str| message(id, 0.0);
        for id in ["m5", "m6"] {
            store.upsert(&said(id)).unwrap();
        }
        // An edit of an old message came through the log and went to the end.
        store.upsert(&said("m3")).unwrap();
        assert_eq!(store.history_before("chat").unwrap(), None);
        store.set_history_before("chat", Some(40)).unwrap();
        assert_eq!(store.history_before("chat").unwrap(), Some(40));

        store.insert_older(&[said("m4"), said("m3")]).unwrap();
        store.insert_older(&[said("m2"), said("m1")]).unwrap();
        store.set_history_before("chat", None).unwrap();
        let ids = |messages: Vec<Message>| messages.into_iter().map(|m| m.id).collect::<Vec<_>>();
        assert_eq!(ids(store.all("chat").unwrap()), ["m1", "m2", "m3", "m4", "m5", "m6"]);
        let (page, more) = store.page("chat", Some("m5"), 2).unwrap();
        assert_eq!((ids(page), more), (vec!["m3".to_string(), "m4".into()], true));
        assert_eq!(store.history_before("chat").unwrap(), None);
    }

    #[test]
    fn upserts_keep_position_and_pages_are_stable() {
        let scratch = scratch();
        for i in 0..5 {
            scratch
                .0
                .upsert(&message(&format!("m{i}"), i as f64))
                .unwrap();
        }
        let (page, more) = scratch.0.page("chat", None, 2).unwrap();
        assert_eq!(
            page.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["m3", "m4"]
        );
        assert!(more);
        let mut changed = message("m3", 3.0);
        changed.state = MessageState::Streaming;
        scratch.0.upsert(&changed).unwrap();
        let (page, _) = scratch
            .0
            .page("chat", Some("m4"), SNAPSHOT_MESSAGES)
            .unwrap();
        assert_eq!(
            page.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["m0", "m1", "m2", "m3"]
        );
    }

    #[test]
    fn context_orders_promoted_messages_and_honors_a_compaction_cursor() {
        let scratch = scratch();
        scratch.0.upsert(&message("first", 1.0)).unwrap();
        let mut promoted = message("steer", 2.0);
        promoted.promoted_at = Some(4.0);
        scratch.0.upsert(&promoted).unwrap();
        scratch.0.upsert(&message("settled", 3.0)).unwrap();
        let (messages, found) = scratch.0.context("chat", None, Some(10)).unwrap();
        assert!(!found);
        assert_eq!(
            messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["first", "settled", "steer"]
        );
        let (messages, found) = scratch.0.context("chat", Some("settled"), Some(1)).unwrap();
        assert!(found);
        assert_eq!(
            messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["steer"]
        );
    }

    #[test]
    fn indexed_activity_queries_do_not_load_tool_payloads() {
        let scratch = scratch();
        let mut user = message("user", 1.0);
        user.body = Body::text("hello");
        let mut own = message("own", 2.0);
        own.author = Author::Bot {
            bot_id: "chef".into(),
        };
        own.body = Body::text("done");
        let mut other = message("other", 3.0);
        other.author = Author::Bot {
            bot_id: "scout".into(),
        };
        other.body = Body::text("found it");
        let mut handoff = message("handoff", 4.0);
        handoff.author = Author::Bot {
            bot_id: "scout".into(),
        };
        handoff.body = Body::Handoff {
            from: "scout".into(),
            to: "chef".into(),
            reason: "take over".into(),
        };
        let mut empty = message("empty", 5.0);
        empty.body = Body::text("  ");
        for row in [user, own, other, handoff, empty] {
            scratch.0.upsert(&row).unwrap();
        }

        assert_eq!(
            scratch.0.last_user_text("chat", "user").unwrap().as_deref(),
            Some("hello")
        );
        assert_eq!(scratch.0.last_user_text("chat", "handoff").unwrap(), None);
        assert_eq!(
            scratch.0.last_bot_text("chat", "chef").unwrap().unwrap().id,
            "own"
        );
        assert_eq!(scratch.0.heard_count("chat", "chef").unwrap(), 4);
        assert_eq!(
            scratch.0.new_count_since_bot_spoke("chat", "chef").unwrap(),
            3
        );
        assert_eq!(
            scratch
                .0
                .text_messages("chat", Some(2), Some(3))
                .unwrap()
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            vec!["own", "other"]
        );
    }

    #[test]
    fn a_turn_reads_the_request_that_started_it() {
        let scratch = scratch();
        let store = &scratch.0;
        let add = |id: &str, author: Author, body: Body| {
            let mut row = message(id, 0.0);
            row.author = author;
            row.body = body;
            store.upsert(&row).unwrap();
        };
        let devops = || Author::Bot {
            bot_id: "devops".into(),
        };
        add("stop", Author::You, Body::text("actually stop that"));
        add("stopped", devops(), Body::text("Stopped."));
        add(
            "handoff",
            Author::Bot {
                bot_id: "chef".into(),
            },
            Body::Handoff {
                from: "chef".into(),
                to: "devops".into(),
                reason: "You own Railway monitoring".into(),
            },
        );
        let card = serde_json::from_value(serde_json::json!({
            "kind": "tool", "name": "bash", "summary": "", "detail": "", "is_running": false
        }))
        .unwrap();
        add("card", devops(), card);
        add(
            "notice",
            Author::System,
            Body::Notice {
                text: "DevOps cannot run yet".into(),
                routine_id: None,
            },
        );
        add(
            "routine",
            Author::System,
            Body::Notice {
                text: "Routine · Railway memory watch".into(),
                routine_id: Some("rt-1".into()),
            },
        );

        let request = |id: &str| store.request_at("chat", id).unwrap().map(|message| message.id);
        assert_eq!(request("stop").as_deref(), Some("stop"));
        assert_eq!(request("stopped").as_deref(), Some("stop"));
        assert_eq!(request("handoff").as_deref(), Some("handoff"));
        // A command's end opens a turn on its card, after the request it served.
        assert_eq!(request("card").as_deref(), Some("handoff"));
        assert_eq!(request("notice").as_deref(), Some("handoff"));
        assert_eq!(request("routine").as_deref(), Some("routine"));
        assert_eq!(request("gone"), None);

        // What a turn did since its request, newest last, and how much a limit left out.
        let after = |id: &str, limit: usize| {
            let (messages, left_out) = store.newest_after("chat", id, limit).unwrap();
            (messages.into_iter().map(|message| message.id).collect::<Vec<_>>(), left_out)
        };
        assert_eq!(after("handoff", 10), (vec!["card".to_string(), "notice".into(), "routine".into()], 0));
        assert_eq!(after("handoff", 2), (vec!["notice".to_string(), "routine".into()], 1));
        assert_eq!(after("gone", 2), (Vec::new(), 0));

        // The user's stop came before the handoff, so the turn it started does not hear it.
        assert_eq!(store.last_user_text("chat", "handoff").unwrap(), None);
        assert_eq!(
            store.last_user_text("chat", "stop").unwrap().as_deref(),
            Some("actually stop that")
        );
        add("steer", Author::You, Body::text("leave Postgres alone"));
        assert_eq!(
            store.last_user_text("chat", "handoff").unwrap().as_deref(),
            Some("leave Postgres alone")
        );
        // Files sent with no words still start the turn, and nothing earlier speaks for them.
        add("files", Author::You, Body::text(""));
        assert_eq!(request("files").as_deref(), Some("files"));
        assert_eq!(store.last_user_text("chat", "files").unwrap(), None);
    }

    #[test]
    fn plain_text_search_scans_visible_message_text() {
        let scratch = scratch();
        let mut deployed = message("deployed", 2.0);
        deployed.body = Body::text("Deployed the resume service successfully");
        scratch.0.upsert(&deployed).unwrap();

        let message_hits = scratch.0.search_messages("deploy resume", 10).unwrap();
        assert_eq!(message_hits[0].message_id, "deployed");
        assert!(message_hits[0].snippet.contains("Deployed"));
        assert!(scratch.0.search_messages("\" OR *", 10).unwrap().is_empty());
        let terms = search_terms("release infra");
        assert!(search_matches("Release Room Infrastructure", &terms));
        assert!(search_snippet("Release Room Infrastructure", &terms).contains("Release"));

        deployed.body = Body::text("Finished something else");
        scratch.0.upsert(&deployed).unwrap();
        assert!(scratch.0.search_messages("deploy", 10).unwrap().is_empty());
        scratch.0.remove("chat", "deployed").unwrap();
        assert!(scratch
            .0
            .search_messages("finished", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn queued_ciphertext_is_binary_and_survives_reopening() {
        let scratch = scratch();
        let ciphertext = vec![0, 255, 128, 13, 10, 34];
        let item = OutboxItem {
            id: "att-file".into(), kind: "file".into(), recipient: None,
            ciphertext: ciphertext.clone(), slot: None, group: Some("chat".into()),
        };
        scratch.0.queue_outbox(&item).unwrap();
        let connection = scratch.0.connection.lock().unwrap();
        let (kind, length): (String, usize) = connection.query_row(
            "SELECT typeof(ciphertext), length(ciphertext) FROM outbox WHERE id = 'att-file'",
            [], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(kind, "blob");
        assert_eq!(length, ciphertext.len());
        drop(connection);
        let reopened = LocalStore::open(&scratch.1.join("beans.sqlite3")).unwrap();
        let queued = reopened.first_outbox().unwrap().unwrap();
        assert_eq!(queued.ciphertext, ciphertext);
        assert_eq!(queued.group.as_deref(), Some("chat"));
    }

    #[test]
    fn a_waiting_slot_is_replaced_without_moving_in_the_outbox() {
        let scratch = scratch();
        let item = |id: &str, slot: Option<&str>| OutboxItem {
            id: id.into(),
            kind: "chat".into(),
            recipient: None,
            ciphertext: id.as_bytes().to_vec(),
            slot: slot.map(|name| Slot {
                name: name.into(),
                keep_first: true,
            }),
            group: Some("chat".into()),
        };
        scratch
            .0
            .queue_outbox(&item("first", Some("message")))
            .unwrap();
        scratch.0.queue_outbox(&item("second", None)).unwrap();
        scratch
            .0
            .queue_outbox(&item("latest", Some("message")))
            .unwrap();

        let outbox = scratch.0.outbox().unwrap();
        assert_eq!(
            outbox
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["latest", "second"]
        );
        assert_eq!(
            scratch.0.first_outbox().unwrap().unwrap().ciphertext,
            b"latest"
        );
    }

    #[test]
    fn metadata_prunes_orphaned_messages_and_grouped_uploads() {
        let scratch = scratch();
        let mut kept = message("kept", 1.0);
        kept.chat_id = "kept-chat".into();
        let mut orphan = message("orphan", 2.0);
        orphan.chat_id = "orphan-chat".into();
        scratch.0.upsert(&kept).unwrap();
        scratch.0.upsert(&orphan).unwrap();
        for (id, group) in [
            ("kept-upload", "kept-chat"),
            ("orphan-upload", "orphan-chat"),
        ] {
            scratch
                .0
                .queue_outbox(&OutboxItem {
                    id: id.into(),
                    kind: "chat".into(),
                    recipient: None,
                    ciphertext: Vec::new(),
                    slot: None,
                    group: Some(group.into()),
                })
                .unwrap();
        }

        scratch.0.retain_chats(&["kept-chat".into()]).unwrap();

        assert!(scratch.0.message("kept-chat", "kept").unwrap().is_some());
        assert!(scratch
            .0
            .message("orphan-chat", "orphan")
            .unwrap()
            .is_none());
        assert_eq!(
            scratch
                .0
                .outbox()
                .unwrap()
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["kept-upload"]
        );
    }
}
