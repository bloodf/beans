//! The relay outbox and the roster baselines a queued roster is rebased on.

use super::*;

pub(crate) fn queue_outbox_tx(tx: &Transaction<'_>, item: &OutboxItem) -> anyhow::Result<()> {
    let waiting: Option<i64> = match item.slot.as_ref() {
        Some(slot) => tx
            .query_row(
                sql!("outbox/slot_position"),
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
                sql!("outbox/replace_in_slot"),
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
                    sql!("outbox/append"),
                    params![item.id, item.kind, item.recipient, item.ciphertext, slot_name, keep_first, item.group],
                )?;
        }
    }
    Ok(())
}

pub(super) fn outbox_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutboxItem> {
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

impl LocalStore {
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
        self.connection.lock().unwrap().execute(sql!("outbox/forget_queued_roster_base"), [])?;
        Ok(())
    }

    pub fn roster_baseline(&self, name: &str) -> anyhow::Result<Option<(i64, crate::model::RosterBlob)>> {
        let connection = self.connection.lock().unwrap();
        let row: Option<(i64, String)> = connection.query_row(
            sql!("outbox/roster_baseline"), [name],
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
            sql!("outbox/save_roster_baseline"),
            params![name, seq, serde_json::to_string(roster)?],
        )?;
        Ok(())
    }

    pub fn advance_queued_roster_base(&self) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute(
            sql!("outbox/advance_queued_roster_base"), [],
        )?;
        Ok(())
    }

    pub fn queue_outbox_with_state(&self, item: &OutboxItem, state: &State) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        if item.kind == "roster" {
            tx.execute(
                sql!("state/queue_observed_roster_base"), [],
            )?;
            tx.execute(
                sql!("state/queue_empty_roster_base"),
                [serde_json::to_string(&crate::model::RosterBlob::default())?],
            )?;
        }
        save_metadata_tx(&tx, state)?;
        sync_json_table(&tx, "device_names", state.device_names.iter()
            .map(|(id, name)| (id.clone(), serde_json::to_string(&(id, name))))
            .collect())?;
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
            sql!("outbox/rebase_queued_roster_with_state_select_outbox"), [], |row| row.get(0),
        ).optional()?;
        if current.as_deref() != Some(expected_id) { return Ok(false); }
        save_metadata_tx(&tx, state)?;
        append_applied_blob_tx(&tx, &item.id)?;
        queue_outbox_tx(&tx, item)?;
        tx.execute(
            sql!("outbox/advance_queued_roster_base"), [],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Drops what the outbox still holds for a relay group: a chat's messages, read marks,
    /// and attachments.
    pub fn drop_outbox_group(&self, group: &str) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute(sql!("messages/delete_group_outbox"), [group])?;
        Ok(())
    }

    pub fn remove_outbox_roster_with_state(&self, id: &str, state: &State, published_chat_ids: &[String]) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        save_metadata_tx(&tx, state)?;
        let removed = tx.execute(sql!("outbox/remove_outbox_roster_with_state_delete_outbox"), [id])?;
        tx.execute(sql!("outbox/remove_outbox_roster_with_state_delete_roster_baseline"), [])?;
        for chat_id in published_chat_ids {
            // A successful PUT publishes these creations even if a newer local edit replaced
            // its outbox slot during the network request.
            tx.execute(sql!("state/acknowledge_chat_creates"), [chat_id])?;
        }
        if removed != 0 {
            tx.execute(sql!("outbox/forget_queued_roster_base"), [])?;
        }
        else {
            tx.execute(
                sql!("outbox/advance_queued_roster_base"), [],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove_outbox_with_state(&self, id: &str, state: &State) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        save_metadata_tx(&tx, state)?;
        tx.execute(sql!("outbox/remove_outbox_roster_with_state_delete_outbox"), [id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn queued_roster(&self) -> anyhow::Result<Option<OutboxItem>> {
        let connection = self.connection.lock().unwrap();
        connection.query_row(
            sql!("outbox/queued_roster"),
            [], outbox_row,
        ).optional().map_err(Into::into)
    }

    pub fn first_outbox(&self) -> anyhow::Result<Option<OutboxItem>> {
        let connection = self.connection.lock().unwrap();
        connection
            .query_row(
                sql!("outbox/first_outbox"),
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
                sql!("outbox/last_outbox"),
                [],
                outbox_row,
            )
            .optional()
            .map_err(Into::into)
    }

    #[cfg(test)]
    pub fn outbox(&self) -> anyhow::Result<Vec<OutboxItem>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            sql!("outbox/outbox"),
        )?;
        let rows = statement.query_map([], outbox_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
}
