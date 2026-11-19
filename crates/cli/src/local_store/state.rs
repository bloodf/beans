//! The account state the app projects into memory (`State`), codemode values, and job journals.

use super::*;

pub(super) fn save_state_tx(tx: &Transaction<'_>, state: &State) -> anyhow::Result<()> {
    reconcile_schedules_tx(tx,state)?;
    save_metadata_tx(tx, state)?;
    sync_json_table(tx, "device_names", state.device_names.iter()
        .map(|(id, name)| (id.clone(), serde_json::to_string(&(id, name))))
        .collect())?;
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
    sync_json_table(
        tx,
        "workflows",
        state
            .workflows
            .iter()
            .map(|workflow| (workflow.id.clone(), serde_json::to_string(workflow)))
            .collect::<Vec<_>>(),
    )?;
    sync_ordered_ids(tx, "group_deletes", &state.group_deletes)?;
    sync_ordered_ids(tx, "blob_deletes", &state.blob_deletes)?;
    sync_ordered_ids(tx, "applied_blobs", &state.applied_blob_ids)?;
    sync_device_seen(tx, &state.device_seen)?;
    Ok(())
}

pub(super) fn save_metadata_tx(tx: &Transaction<'_>, state: &State) -> anyhow::Result<()> {
    tx.execute(
        sql!("state/save_metadata"),
        params![
            serde_json::to_string(&state.auto_review)?,
            state.last_seq,
            state.roster_slot_seq,
            state.machine_blob_hash,
            state.credentials_uploaded,
            state.paused,
            serde_json::to_string(&crate::model::RosterBlob {
                device_name_edits: state.device_name_edits.clone(),
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

pub(super) fn sync_chats(tx: &Transaction<'_>, state: &State) -> anyhow::Result<()> {
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

pub(super) fn append_applied_blob_tx(tx: &Transaction<'_>, id: &str) -> anyhow::Result<()> {
    let position: i64 = tx.query_row(
        sql!("state/next_applied_blob_position"),
        [],
        |row| row.get(0),
    )?;
    tx.execute(
        sql!("state/append_applied_blob"),
        params![id, position],
    )?;
    tx.execute(
        sql!("state/trim_applied_blobs"),
        [],
    )?;
    Ok(())
}

pub(super) fn sync_json_table(
    tx: &Transaction<'_>,
    table: &str,
    rows: Vec<(String, serde_json::Result<String>)>,
) -> anyhow::Result<()> {
    let existing: std::collections::HashMap<String, (i64, String)> = {
        let mut statement = tx.prepare(&for_table(sql!("state/json_rows"), table))?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut retained = std::collections::HashSet::new();
    let upsert = for_table(
        sql!("state/upsert_json_row"),
        table,
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

pub(super) fn sync_ordered_ids(tx: &Transaction<'_>, table: &str, rows: &[String]) -> anyhow::Result<()> {
    let existing: Vec<(String, i64)> = {
        let mut statement = tx.prepare(&for_table(sql!("state/ordered_ids_with_positions"), table))?;
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
    let insert = for_table(sql!("state/insert_ordered_id"), table);
    for id in rows {
        if !existing_ids.contains(id.as_str()) {
            position += 1;
            tx.execute(&insert, params![id, position])?;
        }
    }
    Ok(())
}

pub(super) fn sync_device_seen(
    tx: &Transaction<'_>,
    rows: &std::collections::HashMap<String, i64>,
) -> anyhow::Result<()> {
    let existing: std::collections::HashMap<String, i64> = {
        let mut statement = tx.prepare(sql!("state/device_seen"))?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut retained = std::collections::HashSet::new();
    for (id, seen_at) in rows {
        if existing.get(id) != Some(seen_at) {
            tx.execute(
                sql!("state/upsert_device_seen"),
                params![id, seen_at],
            )?;
        }
        retained.insert(id.clone());
    }
    delete_missing_from(tx, "device_seen", existing.keys(), &retained)
}

pub(super) fn delete_missing_from<'a>(
    tx: &Transaction<'_>,
    table: &str,
    existing: impl Iterator<Item = &'a String>,
    retained: &std::collections::HashSet<String>,
) -> anyhow::Result<()> {
    let delete = for_table(sql!("state/delete_row"), table);
    for id in existing {
        if !retained.contains(id) {
            tx.execute(&delete, [id])?;
        }
    }
    Ok(())
}

pub(super) fn load_json_table<T: DeserializeOwned>(
    connection: &Connection,
    table: &str,
) -> anyhow::Result<Vec<T>> {
    let values: Vec<String> = {
        let mut statement =
            connection.prepare(&for_table(sql!("state/json_values"), table))?;
        let rows = statement.query_map([], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    values
        .into_iter()
        .map(|json| serde_json::from_str(&json).with_context(|| format!("decoding {table} row")))
        .collect()
}

pub(super) fn load_ordered_ids(connection: &Connection, table: &str) -> anyhow::Result<Vec<String>> {
    let mut statement = connection.prepare(&for_table(sql!("state/ordered_ids"), table))?;
    let rows = statement.query_map([], |row| row.get(0))?;
    rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
}

pub(super) fn load_device_turns(
    connection: &Connection,
) -> anyhow::Result<std::collections::HashMap<String, Vec<LiveTurn>>> {
    let mut statement = connection.prepare(sql!("state/load_device_turns"))?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    rows.map(|row| {
        let (id, json) = row?;
        Ok((id, serde_json::from_str(&json).context("decoding a Device's turns")?))
    })
    .collect()
}

pub(super) fn load_device_seen(
    connection: &Connection,
) -> anyhow::Result<std::collections::HashMap<String, i64>> {
    let mut statement = connection.prepare(sql!("state/device_seen"))?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
}

impl LocalStore {
    pub fn load_state(&self) -> anyhow::Result<State> {
        let connection = self.connection.lock().unwrap();
        let metadata: Option<(String, i64, i64, Option<String>, bool, bool, String)> = connection
            .query_row(
                sql!("state/load_state"),
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
            device_name_edits: policy.device_name_edits,
            device_names: load_json_table::<(String, String)>(&connection, "device_names")?.into_iter().collect(),
            bots: load_json_table(&connection, "bots")?,
            chats: load_json_table(&connection, "chats")?,
            routines: load_json_table(&connection, "routines")?,
            workflows: load_json_table(&connection, "workflows")?,
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
        self.save_state_removing_device_turns(state, None, None)
    }

    pub(crate) fn save_state_removing_device_turns(&self, state: &State, device_id: Option<&str>, roster: Option<&OutboxItem>) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        save_state_tx(&tx, state)?;
        if let Some(id) = device_id { tx.execute(sql!("state/delete_device_turns"), [id])?; }
        if let Some(item) = roster {
            tx.execute(sql!("state/queue_observed_roster_base"), [])?;
            tx.execute(sql!("state/queue_empty_roster_base"), [serde_json::to_string(&crate::model::RosterBlob::default())?])?;
            queue_outbox_tx(&tx, item)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn save_state_creating_chat(&self, state: &State, chat: &crate::model::ChatMeta) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        tx.execute(sql!("state/save_state_creating_chat"),
            params![chat.id, serde_json::to_string(chat)?])?;
        save_state_tx(&tx, state)?;
        tx.commit()?;
        Ok(())
    }

    pub fn mark_chat_create_pending(&self, chat: &crate::model::ChatMeta) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute(sql!("state/mark_chat_create_pending"),
            params![chat.id, serde_json::to_string(chat)?])?;
        Ok(())
    }

    pub fn pending_chat_identities(&self) -> anyhow::Result<Vec<(String, Option<crate::model::ChatMeta>)>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(sql!("state/pending_chat_identities"))?;
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
        for id in ids { tx.execute(sql!("state/acknowledge_chat_creates"), [id])?; }
        tx.commit()?;
        Ok(())
    }

    #[cfg(test)]
    pub fn forget_chat_create_identity(&self, id: &str) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute(sql!("state/forget_chat_create_identity"), [id])?;
        Ok(())
    }

    pub fn pending_chat_creates(&self) -> anyhow::Result<Vec<String>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(sql!("state/pending_chat_creates"))?;
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
            tx.execute(sql!("state/delete_chat_receipts"), [chat_id])?;
            tx.execute(sql!("state/delete_chat_invocations"), [chat_id])?;
            tx.execute(sql!("state/delete_chat_tasks"), [chat_id])?;
            tx.execute(sql!("messages/delete_chat"), [chat_id])?;
            tx.execute(sql!("messages/forget_history_before"), [chat_id])?;
            tx.execute(sql!("messages/delete_chat_codemode"), [chat_id])?;
            tx.execute(
                sql!("messages/delete_group_outbox"),
                [crate::model::relay_name(chat_id)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The values a bot's codemode scripts stored in a chat with `store()`.
    pub fn codemode_values(&self, chat_id: &str, bot_id: &str) -> anyhow::Result<std::collections::BTreeMap<String, serde_json::Value>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(sql!("state/codemode_values"))?;
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
            tx.execute(sql!("state/delete_codemode_value"), params![chat_id, bot_id, key])?;
        }
        for (key, value) in set {
            tx.execute(
                sql!("state/upsert_codemode_value"),
                params![chat_id, bot_id, key, serde_json::to_string(value)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// A deleted bot's stored script values, in the chats that outlive it.
    pub fn forget_codemode_values_of(&self, bot_id: &str) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute(sql!("state/forget_codemode_values_of"), [bot_id])?;
        Ok(())
    }

    /// Keeps the stored script values of these bots only: the roster's, once a synced roster
    /// arrives.
    pub fn retain_codemode_bots(&self, bot_ids: &[String]) -> anyhow::Result<()> {
        let valid: std::collections::HashSet<&str> = bot_ids.iter().map(String::as_str).collect();
        let connection = self.connection.lock().unwrap();
        let stored: Vec<String> = {
            let mut statement = connection.prepare(sql!("state/codemode_bots"))?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for bot_id in stored.iter().filter(|bot_id| !valid.contains(bot_id.as_str())) {
            connection.execute(sql!("state/forget_codemode_values_of"), [bot_id])?;
        }
        Ok(())
    }

    /// The turns another Device's latest machine blob lists; none drops its row.
    pub fn set_device_turns(&self, device_id: &str, turns: &[LiveTurn]) -> anyhow::Result<()> {
        let connection = self.connection.lock().unwrap();
        if turns.is_empty() {
            connection.execute(sql!("state/delete_device_turns"), [device_id])?;
        } else {
            connection.execute(
                sql!("state/upsert_device_turns"),
                params![device_id, serde_json::to_string(turns)?],
            )?;
        }
        Ok(())
    }

    /// Keeps a job sealed to another Runner until its wait ends.
    pub fn insert_sent_job(&self, job: &SentJob) -> anyhow::Result<()> {
        let connection = self.connection.lock().unwrap();
        connection.execute(
            sql!("state/insert_sent_job"),
            params![job.id, job.chat_id, job.bot_id, job.routine_id, job.runner_id, job.sent_at],
        )?;
        Ok(())
    }

    pub fn remove_sent_job(&self, id: &str) -> anyhow::Result<()> {
        let connection = self.connection.lock().unwrap();
        connection.execute(sql!("state/remove_sent_job"), [id])?;
        Ok(())
    }

    /// The jobs sealed to other Runners that this Device still waits on, oldest first.
    pub fn sent_jobs(&self) -> anyhow::Result<Vec<SentJob>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            sql!("state/sent_jobs"),
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
}
