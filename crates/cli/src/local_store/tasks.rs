//! Task authority: the fences, leases, invocations, receipts, and routine schedules that decide
//! whether work may run, all written through `LocalStore::safety`.

use super::*;

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

pub(super) fn schedule_unresolved_tx(tx: &Transaction<'_>, epoch: &str, routine: &str) -> anyhow::Result<bool> {
    Ok(tx.query_row(sql!("tasks/schedule_unresolved"), params![epoch,routine], |r| r.get(0))?)
}

pub(super) fn consume_schedule_tx(tx: &Transaction<'_>, lease: &TaskLease, routine: &str, schedule: &ScheduleLease, next: i64) -> anyhow::Result<()> {
    check_authority_tx(tx, lease)?;
    anyhow::ensure!(!schedule_unresolved_tx(tx,&lease.account_epoch,routine)?, "Routine has pending or unknown intent");
    anyhow::ensure!(next > schedule.occurrence, "Invalid next occurrence");
    let changed=tx.execute(sql!("tasks/consume_schedule_update_routine_schedule_authority"),params![lease.account_epoch,routine,schedule.revision,schedule.binding,schedule.occurrence,next])?;
    anyhow::ensure!(changed==1,"Schedule authority changed or occurrence consumed");
    tx.execute(sql!("tasks/consume_schedule_insert_routine_schedule_occurrences"),params![lease.account_epoch,routine,schedule.revision,schedule.occurrence])?;
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

pub(super) fn structural_id(id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')), "Invalid structural identity");
    Ok(())
}

pub(super) fn validate_binding(binding: &InvocationBinding) -> anyhow::Result<()> {
    for id in [&binding.invocation_id, &binding.attempt_id, &binding.receipt_id] { structural_id(id)?; }
    if let Some(id) = &binding.parent_invocation_id { structural_id(id)?; }
    anyhow::ensure!(binding.ordinal <= i64::MAX as u64 && binding.revision <= i64::MAX as u64, "Invocation identity overflow");
    Ok(())
}

pub(super) fn check_authority_tx(tx: &Transaction<'_>, lease: &TaskLease) -> anyhow::Result<()> {
    let valid: bool = tx.query_row(sql!("tasks/check_authority"), params![lease.account_epoch,lease.owner_epoch], |r| r.get(0))?;
    anyhow::ensure!(valid, "Stale execution authority");
    Ok(())
}

pub(super) fn check_running_tx(tx: &Transaction<'_>, lease: &TaskLease) -> anyhow::Result<()> {
    check_authority_tx(tx, lease)?;
    let valid: bool = tx.query_row(sql!("tasks/check_running"), params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id], |r| r.get(0))?;
    anyhow::ensure!(valid, "Task is not admitted for effects");
    Ok(())
}

pub(super) fn queue_task_tx(tx: &Transaction<'_>, lease: &TaskLease, job: &crate::model::Job, local_intent: bool) -> anyhow::Result<bool> {
    check_authority_tx(tx, lease)?;
    for id in [&lease.task_id,&lease.execution_id,&job.chat_id,&job.bot_id] { structural_id(id)?; }
    anyhow::ensure!(lease.task_id == job.id, "Task identity differs from Job.id");
    if let Some(id) = &job.routine_id { structural_id(id)?; }
    // A migrated home lacks a complete historical Job inventory. Sender timestamps
    // cannot establish new intent: all ambiguous admissions stay closed.
    let legacy_closed: bool = tx.query_row(sql!("tasks/legacy_closed"),[],|r|r.get(0))?;
    if legacy_closed && !local_intent {
        tx.execute(sql!("tasks/insert_fence"),params![lease.account_epoch,lease.task_id])?;
        return Ok(false);
    }
    let fresh = tx.execute(sql!("tasks/insert_fresh_fence"), params![lease.account_epoch,lease.task_id])? == 1;
    if !fresh { return Ok(false); }
    tx.execute(sql!("tasks/insert_task"), params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id,job.chat_id,job.bot_id,job.routine_id])?;
    Ok(true)
}

pub(super) fn reconcile_schedules_tx(tx: &Transaction<'_>, state: &State) -> anyhow::Result<()> {
    let authorities: Vec<(String,String)> = tx.prepare(sql!("tasks/reconcile_schedules_select_routine_schedule_authority"))?
        .query_map([],|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    for (id,binding) in authorities {
        let current=state.routines.iter().find(|r|r.id==id).map(|r|schedule_binding(state,r)).transpose()?;
        if current.as_deref()!=Some(binding.as_str()) {
            tx.execute(sql!("tasks/reconcile_schedules_update_routine_schedule_authority"),[id])?;
        }
    }
    Ok(())
}

impl LocalStore {
    pub(crate) fn reauthorize_schedule(&self, lease: &TaskLease, routine: &str, binding: &str, next: i64) -> anyhow::Result<()> {
        self.safety(|tx| {
            check_authority_tx(tx,lease)?;
            anyhow::ensure!(!schedule_unresolved_tx(tx,&lease.account_epoch,routine)?,"Routine has pending or unknown intent");
            tx.execute(sql!("tasks/reauthorize_schedule"),params![lease.account_epoch,routine,uuid::Uuid::new_v4().to_string(),binding,next])?;
            Ok(())
        })
    }

    pub(crate) fn revoke_schedule(&self, routine: &str) -> anyhow::Result<()> {
        self.safety(|tx| { tx.execute(sql!("tasks/reconcile_schedules_update_routine_schedule_authority"),[routine])?; Ok(()) })
    }

    pub(crate) fn reconcile_schedules(&self, state: &State) -> anyhow::Result<()> {
        self.safety(|tx| reconcile_schedules_tx(tx,state))
    }

    pub(crate) fn schedule_lease(&self, routine: &str, binding: &str, now: i64, next: i64) -> anyhow::Result<Option<ScheduleLease>> {
        self.safety(|tx| {
            // Skip elapsed occurrences, never dispatch a missed-tick backlog.
            tx.execute(sql!("tasks/schedule_lease_update_routine_schedule_authority"),params![routine,binding,now.saturating_sub(30),next])?;
            Ok(tx.query_row(sql!("tasks/schedule_lease_select_routine_schedule_authority"),params![routine,binding,now],|r|Ok(ScheduleLease{revision:r.get(0)?,binding:r.get(1)?,occurrence:r.get(2)?})).optional()?)
        })
    }

    pub(crate) fn commit_scheduled_occurrence(&self, lease: &TaskLease, job: Option<&crate::model::Job>, routine: &str, schedule: &ScheduleLease, next: i64, chat: &str, bot: &str, at: i64, set: &std::collections::BTreeMap<String,serde_json::Value>, delete: &[String]) -> anyhow::Result<()> {
        self.safety(|tx| {
            consume_schedule_tx(tx,lease,routine,schedule,next)?;
            if let Some(job)=job {
                anyhow::ensure!(queue_task_tx(tx,lease,job,true)?,"Routine task already fenced");
                tx.execute(sql!("tasks/commit_scheduled_occurrence_update_local_tasks"),params![lease.account_epoch,lease.task_id,serde_json::to_string(&job.check)?])?;
            }
            for key in delete { tx.execute(sql!("tasks/commit_scheduled_occurrence_delete_codemode_store"),params![chat,bot,key])?; }
            for (key,value) in set { tx.execute(sql!("tasks/commit_scheduled_occurrence_insert_codemode_store"),params![chat,bot,key,serde_json::to_string(value)?])?; }
            tx.execute(sql!("tasks/commit_scheduled_occurrence_insert_routine_check_commits"),params![lease.account_epoch,routine,at])?;
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
        Ok(self.connection.lock().unwrap().query_row(sql!("tasks/legacy_closed"),[],|r|r.get(0)).optional()?.unwrap_or(false))
    }

    pub(crate) fn history_task_lease(&self, owner: &str, task: &str, incarnation: u64) -> anyhow::Result<Option<TaskLease>> {
        Ok(self.connection.lock().unwrap().query_row(sql!("tasks/history_task_lease"),params![task,owner],|r|Ok(TaskLease {account_epoch:r.get(0)?,owner_epoch:r.get(1)?,execution_id:r.get(2)?,task_id:task.into(),incarnation})).optional()?)
    }

    pub(crate) fn active_task_lease(&self, owner: &str, task: &str, incarnation: u64) -> anyhow::Result<Option<TaskLease>> {
        Ok(self.connection.lock().unwrap().query_row(sql!("tasks/active_task_lease"),params![owner,task],|r| Ok(TaskLease { account_epoch:r.get(0)?,execution_id:r.get(1)?,owner_epoch:owner.into(),task_id:task.into(),incarnation })).optional()?)
    }

    pub(crate) fn commit_routine_check(&self, lease: &TaskLease, job: Option<&crate::model::Job>, routine: &str, chat: &str, bot: &str, at: i64, set: &std::collections::BTreeMap<String,serde_json::Value>, delete: &[String]) -> anyhow::Result<bool> {
        self.safety(|tx| {
            check_authority_tx(tx,lease)?;
            if let Some(job) = job {
                anyhow::ensure!(queue_task_tx(tx,lease,job,false)?, "Routine task already exists");
                tx.execute(sql!("tasks/commit_scheduled_occurrence_update_local_tasks"),params![lease.account_epoch,lease.task_id,serde_json::to_string(&job.check)?])?;
            }
            for key in delete { tx.execute(sql!("tasks/commit_scheduled_occurrence_delete_codemode_store"),params![chat,bot,key])?; }
            for (key,value) in set { tx.execute(sql!("tasks/commit_scheduled_occurrence_insert_codemode_store"),params![chat,bot,key,serde_json::to_string(value)?])?; }
            tx.execute(sql!("tasks/commit_scheduled_occurrence_insert_routine_check_commits"),params![lease.account_epoch,routine,at])?;
            Ok(true)
        })
    }

    pub(crate) fn routine_checked_at(&self, routine: &str) -> anyhow::Result<Option<i64>> {
        Ok(self.connection.lock().unwrap().query_row(sql!("tasks/routine_checked_at"),[routine],|r|r.get(0)).optional()?)
    }

    pub(crate) fn current_task_account_epoch(&self) -> anyhow::Result<Option<String>> {
        Ok(self.connection.lock().unwrap().query_row(sql!("tasks/current_task_account_epoch"), [], |r| r.get(0)).optional()?)
    }

    pub(crate) fn task_account_epoch(&self, owner: &str) -> anyhow::Result<String> {
        self.safety(|tx| {
            let authority: Option<(String,String,bool)> = tx.query_row(sql!("tasks/task_account_epoch_select_task_authority"), [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            match authority {
                Some((epoch,current,false)) if current == owner => Ok(epoch),
                Some(_) => anyhow::bail!("Stale task owner"),
                None => {
                    let epoch = uuid::Uuid::new_v4().to_string();
                    tx.execute(sql!("tasks/task_account_epoch_insert_task_authority"), params![epoch,owner])?;
                    tx.execute(sql!("tasks/task_account_epoch_insert_task_safety_version"),[])?;
                    Ok(epoch)
                }
            }
        })
    }

    /// All safety mutations use this boundary. Keep FULL for the connection lifetime so
    /// unrelated state/policy writes cannot weaken a committed execution fence.
    pub(super) fn safety<T>(&self, write: impl FnOnce(&Transaction<'_>) -> anyhow::Result<T>) -> anyhow::Result<T> {
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
                sql!("tasks/recover_task_owner_select_task_authority"), [], |r| Ok((r.get(0)?, r.get(1)?)),
            ).optional()?;
            let epoch = match authority {
                Some((epoch, false)) => epoch,
                Some((_, true)) => anyhow::bail!("Task account is closed"),
                None => {
                    let orphaned: bool = tx.query_row(sql!("tasks/recover_task_owner_select_local_tasks"), [], |r| r.get(0))?;
                    anyhow::ensure!(!orphaned, "Task authority is missing from a store containing safety records");
                    let migrated: bool = tx.query_row(sql!("tasks/recover_task_owner_select_task_safety_version"),[],|r|r.get(0))?;
                    anyhow::ensure!(!migrated, "Migrated task authority is missing; automatic reconstruction is forbidden");
                    let epoch = uuid::Uuid::new_v4().to_string();
                    let legacy: bool = tx.query_row(sql!("tasks/recover_task_owner_select_bots"),[],|r|r.get(0))?;
                    let floor = legacy.then_some(1.0);
                    tx.execute(sql!("tasks/recover_task_owner_insert_task_authority"), params![epoch, owner,floor])?;
                    tx.execute(sql!("tasks/task_account_epoch_insert_task_safety_version"),[])?;
                    // Available legacy Job identities are denial evidence only. Sent jobs
                    // remain pending remote-result waits; migration never dispatches them.
                    tx.execute(sql!("tasks/fence_legacy_sent_jobs"), [&epoch])?;
                    tx.execute(sql!("tasks/fence_legacy_memory_admissions"), [&epoch])?;
                    let turns: Vec<String> = tx.prepare(sql!("tasks/recover_task_owner_select_device_turns"))?
                        .query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                    for json in turns {
                        let turns: Vec<LiveTurn> = serde_json::from_str(&json).context("Invalid legacy turn journal")?;
                        for turn in turns {
                            structural_id(&turn.job_id)?;
                            tx.execute(sql!("tasks/insert_fence"),params![epoch,turn.job_id])?;
                        }
                    }
                    epoch
                }
            };
            tx.execute(sql!("tasks/recover_task_owner_update_task_effect_receipts"), [&epoch])?;
            tx.execute(sql!("tasks/recover_task_owner_update_task_invocations"), [&epoch])?;
            tx.execute(sql!("tasks/recover_task_owner_update_local_tasks"), [&epoch])?;
            tx.execute(sql!("tasks/recover_task_owner_update_task_authority"), params![owner, epoch])?;
            Ok(epoch)
        })
    }

    /// A non-executing Device may durably refuse an envelope without creating authority.
    pub(crate) fn quarantine_task_envelope(&self, identity: &str, task: &str) -> anyhow::Result<()> {
        structural_id(identity)?;
        structural_id(task)?;
        self.safety(|tx| {
            tx.execute(sql!("tasks/quarantine_task_envelope"), params![identity,task])?;
            Ok(())
        })
    }

    pub(crate) fn queue_task_for_identity(&self, lease: &TaskLease, job: &crate::model::Job, identity: &str) -> anyhow::Result<bool> {
        structural_id(identity)?;
        self.safety(|tx| {
            check_authority_tx(tx, lease)?;
            structural_id(&job.id)?;
            anyhow::ensure!(lease.task_id == job.id, "Task identity differs from Job.id");
            let rejected: bool = tx.query_row(sql!("tasks/queue_task_for_identity_select_task_rejected_envelopes"), params![identity,job.id], |row| row.get(0))?;
            if rejected {
                tx.execute(sql!("tasks/insert_fence"), params![lease.account_epoch,lease.task_id])?;
                return Ok(false);
            }
            queue_task_tx(tx, lease, job, false)
        })
    }

    #[cfg(test)]
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
            Ok(tx.execute(sql!("tasks/start_task"),
                params![lease.account_epoch, lease.task_id, lease.owner_epoch, lease.execution_id])? == 1)
        })
    }

    /// The worker has relinquished this exact queued execution; replay remains fenced.
    pub(crate) fn abandon_queued_task(&self, lease: &TaskLease) -> anyhow::Result<bool> {
        self.safety(|tx| {
            check_authority_tx(tx, lease)?;
            Ok(tx.execute(sql!("tasks/abandon_queued_task"),
                params![lease.account_epoch, lease.task_id, lease.owner_epoch, lease.execution_id])? == 1)
        })
    }

    pub(crate) fn finish_task(&self, lease: &TaskLease, interrupted: bool) -> anyhow::Result<bool> {
        self.safety(|tx| {
            check_authority_tx(tx, lease)?;
            Ok(tx.execute(sql!("tasks/finish_task"),
                params![lease.account_epoch, lease.task_id, lease.owner_epoch, lease.execution_id, interrupted])? == 1)
        })
    }

    pub fn task_state(&self, lease: &TaskLease) -> anyhow::Result<Option<TaskState>> {
        let value: Option<String> = self.connection.lock().unwrap().query_row(
            sql!("tasks/task_state"),
            params![lease.account_epoch, lease.task_id, lease.owner_epoch, lease.execution_id], |r| r.get(0),
        ).optional()?;
        value.as_deref().map(TaskState::parse).transpose()
    }

    pub(crate) fn cancel_tasks(&self, epoch: &str, task: Option<&str>, chat: Option<&str>, bot: Option<&str>) -> anyhow::Result<()> {
        self.safety(|tx| {
            if let Some(id) = task {
                structural_id(id)?;
                tx.execute(sql!("tasks/cancel_tasks_insert_task_fences"), params![epoch,id])?;
            }
            tx.execute(sql!("tasks/cancel_tasks_update_local_tasks"), params![epoch,task,chat,bot])?;
            tx.execute(sql!("tasks/cancel_tasks_update_task_invocations"), [epoch])?;
            Ok(())
        })
    }

    pub(crate) fn close_task_account(&self) -> anyhow::Result<()> {
        self.safety(|tx| {
            tx.execute(sql!("tasks/close_task_account_update_task_authority"), [])?;
            tx.execute(sql!("tasks/close_task_account_update_local_tasks"), [])?;
            tx.execute(sql!("tasks/close_task_account_update_task_invocations"), [])?;
            Ok(())
        })
    }

    pub(crate) fn routine_needs_review(&self, epoch: &str, routine: &str) -> anyhow::Result<bool> {
        Ok(self.connection.lock().unwrap().query_row(sql!("tasks/routine_needs_review"), params![epoch,routine], |r| r.get(0))?)
    }

    /// Freeze once; changing a binding is a different invocation, not an UPDATE.
    pub(crate) fn insert_invocation(&self, lease: &TaskLease, binding: &InvocationBinding) -> anyhow::Result<bool> {
        validate_binding(binding)?;
        self.safety(|tx| {
            check_running_tx(tx, lease)?;
            Ok(tx.execute(sql!("tasks/insert_invocation"),
                params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id,binding.invocation_id,binding.parent_invocation_id,binding.ordinal,binding.attempt_id,binding.receipt_id,binding.revision,binding.digest.as_slice()])? == 1)
        })
    }

    pub(crate) fn decide_invocation(&self, lease: &TaskLease, binding: &InvocationBinding, decision: AuthorizationDecision, kind: AuthorizationKind, card: Option<&str>, device: Option<&str>, at: i64) -> anyhow::Result<bool> {
        validate_binding(binding)?;
        if let Some(id) = card { structural_id(id)?; }
        if let Some(id) = device { structural_id(id)?; }
        self.safety(|tx| {
            check_running_tx(tx, lease)?;
            Ok(tx.execute(sql!("tasks/decide_invocation"),
                params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id,binding.invocation_id,binding.parent_invocation_id,binding.ordinal,binding.attempt_id,binding.receipt_id,binding.revision,binding.digest.as_slice(),decision.as_str(),kind.as_str(),card,device,at])? == 1)
        })
    }

    /// Called only after the host's protected policy recheck; successful commit is the
    /// single dispatch permission. Any error, including ambiguous acknowledgment, forbids I/O.
    pub(crate) fn admit_invocation(&self, lease: &TaskLease, binding: &InvocationBinding) -> anyhow::Result<bool> {
        validate_binding(binding)?;
        self.safety(|tx| {
            check_running_tx(tx, lease)?;
            let changed = tx.execute(sql!("tasks/admit_invocation_update_task_invocations"),
                params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id,binding.invocation_id,binding.parent_invocation_id,binding.ordinal,binding.attempt_id,binding.receipt_id,binding.revision,binding.digest.as_slice()])?;
            if changed == 0 { return Ok(false); }
            tx.execute(sql!("tasks/admit_invocation_insert_task_effect_receipts"),
                params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id,binding.attempt_id,binding.receipt_id])?;
            Ok(true)
        })
    }

    pub(crate) fn finish_receipt(&self, lease: &TaskLease, binding: &InvocationBinding, outcome: ReceiptOutcome) -> anyhow::Result<bool> {
        validate_binding(binding)?;
        self.safety(|tx| {
            check_authority_tx(tx, lease)?;
            Ok(tx.execute(sql!("tasks/finish_receipt"),
                params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id,binding.attempt_id,binding.receipt_id,outcome.as_str(),binding.invocation_id,binding.revision,binding.digest.as_slice()])? == 1)
        })
    }

    pub(crate) fn resolve_task(&self, lease: &TaskLease, at: i64) -> anyhow::Result<bool> {
        self.safety(|tx| {
            let current:bool=tx.query_row(sql!("tasks/resolve_task_select_task_authority"),[&lease.account_epoch],|r|r.get(0))?;
            anyhow::ensure!(current,"Stale task account");
            Ok(tx.execute(sql!("tasks/resolve_task_update_local_tasks"),
                params![lease.account_epoch,lease.task_id,lease.owner_epoch,lease.execution_id,at])? == 1)
        })
    }

    pub(crate) fn prune_tasks(&self, now: i64) -> anyhow::Result<()> {
        self.safety(|tx| {
            let cutoff = now.saturating_sub(30 * 24 * 60 * 60);
            // Routine unknown receipts remain admission evidence even after review resolution.
            tx.execute(sql!("tasks/prune_tasks_delete_task_effect_receipts"), [cutoff])?;
            tx.execute(sql!("tasks/prune_tasks_delete_task_invocations"), [cutoff])?;
            tx.execute(sql!("tasks/prune_tasks_delete_local_tasks"), [cutoff])?;
            Ok(())
        })
    }
}
