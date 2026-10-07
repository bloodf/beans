//! Account-owned memory foundation, separate from local MEMORY.md and inference providers.
pub mod types;
pub mod config;
pub mod queue;
pub mod api;
#[cfg(feature = "runner")]
pub mod admission;
#[cfg(feature = "runner")]
pub mod dispatch;
#[cfg(feature = "runner")]
pub mod backends;
#[cfg(feature = "runner")]
pub mod setup;
#[cfg(feature = "runner")]
pub mod tools;

use crate::app::{App, OutboxItem, Slot};
use config::MemoryConfig;
use queue::{DeletionFence, Delivery};
use types::MemoryError;

pub fn load(store: &crate::local_store::LocalStore, dek: Option<[u8; 32]>) -> anyhow::Result<MemoryConfig> {
    match store.memory_ciphertext()? {
        Some(ciphertext) => {
            let dek = dek.ok_or_else(|| anyhow::anyhow!("memory config exists without account key"))?;
            let config: MemoryConfig = crate::crypto::decrypt_json(&dek, "memory_config", &ciphertext)?;
            config.validate()?; Ok(config)
        },
        None => Ok(MemoryConfig::default()),
    }
}

pub fn delivery_valid(config: &MemoryConfig, d: &Delivery) -> bool {
    let Some(bot) = config.bots.get(&d.bot_id).and_then(|r| r.value.as_ref()) else { return false; };
    let Some(connection) = config.connections.get(&d.scope.connection_id) else { return false; };
    connection.value.is_some() && connection.revision == d.scope.connection_revision
        && bot.connection_id.as_deref() == Some(&d.scope.connection_id)
        && bot.deletion_epoch == d.scope.deletion_epoch
        && d.consent_revision.as_ref().is_none_or(|r| bot.capture_conversation && &bot.consent_revision == r)
}
impl App {
    /// Caller holds roster_edit. Bot removal disables future delivery without deleting a bank.
    pub(crate) fn invalidate_bot_memory(&self, bot_id: &str) -> Result<(),MemoryError> {
        let mut current=self.memory_config.lock();
        let Some(mut preferences)=current.bots.get(bot_id).and_then(|r|r.value.clone()) else { return Ok(()); };
        if preferences.connection_id.is_none() && !preferences.capture_conversation && !preferences.auto_recall { return Ok(()); }
        preferences.connection_id=None;preferences.capture_conversation=false;preferences.capture_group_text=false;
        preferences.auto_recall=false;preferences.unattended_capture=false;
        preferences.deletion_epoch=preferences.deletion_epoch.checked_add(1).ok_or_else(||MemoryError::new("epoch_exhausted"))?;
        let mut next=current.clone();
        next.set_bot(bot_id,preferences,&self.this_device_id().ok_or_else(||MemoryError::new("identity_required"))?)?;
        self.persist_memory(&next,true,None)?;*current=next;
        Ok(())
    }
    /// Callers hold memory_config through persistence and publish only after the transaction.
    pub(crate) fn persist_memory(&self, config: &MemoryConfig, publish: bool, fence: Option<&DeletionFence>) -> Result<(), MemoryError> {
        config.validate()?;
        let dek = self.dek().ok_or_else(|| MemoryError::new("identity_required"))?;
        let ciphertext = crate::crypto::encrypt_json(&dek, "memory_config", config).map_err(|_| MemoryError::new("memory_encryption_failed"))?;
        let outbox = publish.then(|| OutboxItem { id: format!("mem-{}", uuid::Uuid::new_v4()), kind: "memory_config".into(),
            recipient: None, ciphertext: ciphertext.clone(), slot: Some(Slot::latest("memory_config")), group: None });
        self.store.save_memory_config(&ciphertext, outbox.as_ref(), |d| delivery_valid(config, d), fence)
            .map_err(|_| MemoryError::new("memory_storage_failed"))?;
        #[cfg(feature = "runner")]
        self.memory_runtime.cancel_invalid(config);
        if publish { self.outbox_notify.notify_one(); }
        Ok(())
    }
    pub fn apply_memory_config(&self, incoming: &MemoryConfig) -> Result<(), MemoryError> {
        let mut current = self.memory_config.lock();
        let mut next = current.clone();
        next.merge(incoming)?;
        let publish = &next != incoming;
        self.persist_memory(&next, publish, None)?;
        *current = next;
        drop(current);
        self.memory_changed(None, "config_changed");
        Ok(())
    }
    pub fn push_memory_config(&self) -> Result<(), MemoryError> {
        let current = self.memory_config.lock();
        if current.clock != 0 { self.persist_memory(&current, true, None)?; }
        Ok(())
    }
    pub fn memory_changed(&self, bot_id: Option<&str>, status: &str) {
        self.emit(crate::events::Event::MemoryChanged { bot_id: bot_id.map(str::to_owned), status: status.into() });
    }
    pub fn memory_summary(&self) -> serde_json::Value {
        self.memory_config.lock().summary()
    }
}
