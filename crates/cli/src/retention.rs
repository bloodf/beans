//! Read-only local transcript planning. A plan never authorizes deletion.

use crate::local_store::LocalStore;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum RetentionPolicy {
    #[default]
    Disabled,
    /// Explicit Unix-seconds cutoff, exclusive. No implicit clock or global scope.
    Before(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatAdmission {
    /// A caller's snapshot only; execution would require a fresh owner fence.
    OwnedAndIdle,
    Active,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionBlocker {
    Disabled,
    OwnershipOrActivityUnknown,
    ActiveTurn,
    InvalidScopeOrCutoff,
    /// Attachments, derived memory, relay and safety records are not reconciled.
    InventoryIncomplete,
}

#[derive(Debug, PartialEq)]
pub struct RetentionPlan {
    pub chat_id: String,
    pub policy: RetentionPolicy,
    /// Local display-history candidates only. No bodies, paths or tool arguments.
    pub message_ids: Vec<String>,
    /// Every plan currently has a blocker. There is no deletion executor.
    pub blockers: Vec<RetentionBlocker>,
}

impl LocalStore {
    /// Reads one exact local chat. Does not persist settings, enqueue deletes,
    /// emit a deletion receipt or change account-lifetime replay fences.
    pub fn retention_dry_run(
        &self,
        chat_id: &str,
        policy: RetentionPolicy,
        admission: ChatAdmission,
    ) -> anyhow::Result<RetentionPlan> {
        let mut plan = RetentionPlan {
            chat_id: chat_id.into(),
            policy,
            message_ids: Vec::new(),
            blockers: vec![RetentionBlocker::InventoryIncomplete],
        };
        let cutoff = match policy {
            RetentionPolicy::Disabled => {
                plan.blockers.push(RetentionBlocker::Disabled);
                return Ok(plan);
            }
            RetentionPolicy::Before(at)
                if at.is_finite() && at > 0.0 && !chat_id.trim().is_empty() =>
            {
                at
            }
            _ => {
                plan.blockers.push(RetentionBlocker::InvalidScopeOrCutoff);
                return Ok(plan);
            }
        };
        match admission {
            ChatAdmission::OwnedAndIdle => {}
            ChatAdmission::Active => {
                plan.blockers.push(RetentionBlocker::ActiveTurn);
                return Ok(plan);
            }
            ChatAdmission::Unknown => {
                plan.blockers
                    .push(RetentionBlocker::OwnershipOrActivityUnknown);
                return Ok(plan);
            }
        }
        for message in self.all(chat_id)? {
            // Incomplete/queued rows and invalid timestamps cannot become candidates.
            if message.chat_id != chat_id {
                anyhow::bail!("retention chat scope mismatch");
            }
            if message.is_complete()
                && !message.queued
                && message.created_at.is_finite()
                && message.created_at >= 0.0
                && message.created_at < cutoff
                && message
                    .promoted_at
                    .is_none_or(|at| at.is_finite() && at >= 0.0 && at < cutoff)
            {
                plan.message_ids.push(message.id);
            }
        }
        Ok(plan)
    }
}
