//! Durable frozen delivery intent. Service traffic never runs from the relay outbox.
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use super::types::*;
use crate::local_store::LocalStore;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Delivery {
    pub id: String, pub bot_id: String, pub scope: MemoryScope,
    pub consent_revision: Option<Revision>, pub document: FrozenDocument,
    pub state: OperationState, pub operation_id: Option<String>, pub error_code: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeletionFence {
    pub bot_id: String, pub scope: MemoryScope, pub document_id: Option<String>,
    pub request_id: String, pub pending: bool, pub operation_id: Option<String>,
}

pub fn freeze_document(namespace: &str, text: &str, sources: Vec<Source>) -> Result<FrozenDocument, MemoryError> {
    if text.is_empty() || text.len() > 32768 || sources.len() > 128 {
        return Err(MemoryError::new("invalid_document"));
    }
    let text = crate::memory::scrub(text);
    if text.len() > 32768 { return Err(MemoryError::new("invalid_document")); }
    let content_hash = digest(text.as_bytes());
    let sources_json = serde_json::to_vec(&sources).map_err(|_| MemoryError::new("invalid_document"))?;
    let mut input = Vec::with_capacity(namespace.len() + sources_json.len() + content_hash.len() + 32);
    input.extend_from_slice(b"beans.memory.document.v1\0");
    input.extend_from_slice(namespace.as_bytes()); input.push(0);
    input.extend_from_slice(&sources_json); input.push(0); input.extend_from_slice(content_hash.as_bytes());
    let id = digest(&input);
    input.extend_from_slice(b"\0request");
    Ok(FrozenDocument { id, request_id: digest(&input), text, sources, content_hash })
}
fn storage(error: impl std::fmt::Display) -> MemoryError {
    tracing::error!(%error, "memory queue storage"); MemoryError::new("memory_storage_failed")
}
impl LocalStore {
    pub fn save_memory_binding(&self, key: &str, json: &str) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute(
            "INSERT INTO memory_runner_bindings(key,json) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET json=excluded.json",
            params![key,json])?;
        Ok(())
    }
    pub fn memory_binding(&self, key: &str) -> anyhow::Result<Option<String>> {
        Ok(self.connection.lock().unwrap().query_row("SELECT json FROM memory_runner_bindings WHERE key=?1",
            [key], |r| r.get(0)).optional()?)
    }
    pub fn memory_has_uncertain(&self, scope: &MemoryScope) -> Result<bool, MemoryError> {
        self.connection.lock().unwrap().query_row(
            "SELECT EXISTS(SELECT 1 FROM memory_deliveries WHERE state IN ('submitted','processing','delivery_unknown')
             AND json_extract(json,'$.scope.namespace')=?1 AND json_extract(json,'$.scope.connection_id')=?2)",
            params![scope.namespace, scope.connection_id], |row| row.get(0)).map_err(storage)
    }
    pub fn memory_ciphertext(&self) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(self.connection.lock().unwrap().query_row("SELECT ciphertext FROM memory_config WHERE id=1", [], |r| r.get(0)).optional()?)
    }
    /// Local encrypted config, ciphertext outbox and invalidated delivery intent commit together.
    pub fn save_memory_config(&self, ciphertext: &[u8], outbox: Option<&crate::app::OutboxItem>,
        valid: impl Fn(&Delivery) -> bool, fence: Option<&DeletionFence>) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        tx.execute("INSERT INTO memory_config(id,ciphertext) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET ciphertext=excluded.ciphertext", [ciphertext])?;
        if let Some(item) = outbox { crate::local_store::queue_outbox_tx(&tx, item)?; }
        let rows: Vec<(String, String)> = tx.prepare("SELECT id,json FROM memory_deliveries WHERE state != 'completed'")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        for (id, json) in rows {
            let mut d: Delivery = serde_json::from_str(&json)?;
            if !valid(&d) {
                d.error_code = Some("authority_changed".into());
                d.state = if matches!(d.state, OperationState::Submitted | OperationState::Processing | OperationState::DeliveryUnknown) {
                    OperationState::DeliveryUnknown
                } else { OperationState::Failed };
                tx.execute("UPDATE memory_deliveries SET state=?2,json=?3 WHERE id=?1", params![id, state_name(d.state), serde_json::to_string(&d)?])?;
            }
        }
        if let Some(f) = fence {
            tx.execute("INSERT INTO memory_fences(bot_id,json) VALUES(?1,?2) ON CONFLICT(bot_id) DO UPDATE SET json=excluded.json", params![f.bot_id, serde_json::to_string(f)?])?;
        }
        tx.commit()?; Ok(())
    }
    pub fn enqueue_memory(&self, delivery: &Delivery) -> Result<(), MemoryError> {
        let connection = self.connection.lock().unwrap();
        let existing: Option<String> = connection.query_row("SELECT json FROM memory_deliveries WHERE id=?1", [&delivery.id], |r| r.get(0)).optional().map_err(storage)?;
        if let Some(existing) = existing {
            let old: Delivery = serde_json::from_str(&existing).map_err(storage)?;
            if old.document != delivery.document || old.bot_id != delivery.bot_id || old.scope != delivery.scope {
                return Err(MemoryError::new("request_id_conflict"));
            }
            return Ok(());
        }
        connection.execute("INSERT INTO memory_deliveries(id,bot_id,state,json) VALUES(?1,?2,?3,?4)",
            params![delivery.id, delivery.bot_id, state_name(delivery.state), serde_json::to_string(delivery).map_err(storage)?]).map_err(storage)?;
        Ok(())
    }
    pub fn memory_deliveries(&self, bot_id: &str) -> Result<Vec<Delivery>, MemoryError> {
        let connection = self.connection.lock().unwrap();
        let mut stmt = connection.prepare("SELECT json FROM memory_deliveries WHERE bot_id=?1 ORDER BY (state!='completed') DESC,rowid DESC LIMIT 1000").map_err(storage)?;
        let rows = stmt.query_map([bot_id], |r| r.get::<_, String>(0)).map_err(storage)?;
        rows.map(|r| serde_json::from_str(&r.map_err(storage)?).map_err(storage)).collect()
    }
    pub fn memory_delivery(&self, bot: &str, id: &str) -> Result<Delivery, MemoryError> {
        let json: Option<String> = self.connection.lock().unwrap().query_row("SELECT json FROM memory_deliveries WHERE bot_id=?1 AND id=?2",
            params![bot, id], |r| r.get(0)).optional().map_err(storage)?;
        serde_json::from_str(&json.ok_or_else(|| MemoryError::new("operation_not_found"))?).map_err(storage)
    }
    /// CAS applies only to the exact frozen row observed by the caller (fences win late replies).
    pub fn transition_memory(&self, before: &Delivery, after: &Delivery) -> Result<bool, MemoryError> {
        let changed = self.connection.lock().unwrap().execute("UPDATE memory_deliveries SET state=?3,json=?4 WHERE id=?1 AND json=?2",
            params![before.id, serde_json::to_string(before).map_err(storage)?, state_name(after.state), serde_json::to_string(after).map_err(storage)?]).map_err(storage)?;
        Ok(changed == 1)
    }
    pub fn recover_memory_queue(&self) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        let rows: Vec<(String, String)> = tx.prepare("SELECT id,json FROM memory_deliveries WHERE state='submitted'")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        for (id, json) in rows {
            let mut d: Delivery = serde_json::from_str(&json)?;
            d.state = OperationState::DeliveryUnknown; d.error_code = Some("response_lost".into());
            tx.execute("UPDATE memory_deliveries SET state='delivery_unknown',json=?2 WHERE id=?1", params![id, serde_json::to_string(&d)?])?;
        }
        tx.commit()?; Ok(())
    }
    pub fn memory_fence(&self, bot: &str) -> Result<Option<DeletionFence>, MemoryError> {
        let json: Option<String> = self.connection.lock().unwrap().query_row("SELECT json FROM memory_fences WHERE bot_id=?1", [bot], |r| r.get(0)).optional().map_err(storage)?;
        json.map(|j| serde_json::from_str(&j).map_err(storage)).transpose()
    }
    pub fn update_memory_fence(&self, before: &DeletionFence, after: &DeletionFence) -> Result<bool, MemoryError> {
        Ok(self.connection.lock().unwrap().execute("UPDATE memory_fences SET json=?3 WHERE bot_id=?1 AND json=?2",
            params![before.bot_id, serde_json::to_string(before).map_err(storage)?, serde_json::to_string(after).map_err(storage)?]).map_err(storage)? == 1)
    }
}
fn state_name(state: OperationState) -> &'static str {
    match state { OperationState::Queued => "queued", OperationState::Submitted => "submitted", OperationState::Processing => "processing",
        OperationState::Completed => "completed", OperationState::Failed => "failed", OperationState::DeliveryUnknown => "delivery_unknown" }
}
