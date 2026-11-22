//! Attention rows: each item, brief, resolution and preference stays encrypted at rest, and the
//! one-time upgrade replay marker for slots an older build consumed without recognizing them.

use super::*;

impl LocalStore {
    /// Every attention ciphertext, by id.
    pub fn attention_rows(&self) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(sql!("attention/rows"))?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// One attention ciphertext by id, if present.
    pub fn attention_row(&self, id: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let connection = self.connection.lock().unwrap();
        connection.query_row(sql!("attention/row"), params![id], |row| row.get(0)).optional().map_err(Into::into)
    }

    pub fn put_attention(&self, id: &str, ciphertext: &[u8]) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute(sql!("attention/put"), params![id, ciphertext])?;
        Ok(())
    }

    /// Whether this Device has finished the one-time attention-slot upgrade replay.
    pub fn attention_upgrade_done(&self) -> anyhow::Result<bool> {
        let connection = self.connection.lock().unwrap();
        let done: i64 = connection.query_row(sql!("attention/upgrade_done"), [], |row| row.get(0)).optional()?.unwrap_or(0);
        Ok(done != 0)
    }

    /// Persists upgrade-replay completion. Upserts the metadata row when missing so sync never
    /// fails on this path; the row it creates still loads. Cleared with the account (`clear`).
    pub fn set_attention_upgrade_done(&self, done: bool) -> anyhow::Result<()> {
        let connection = self.connection.lock().unwrap();
        let flag = done as i64;
        if connection.execute(sql!("attention/set_upgrade_done"), params![flag])? > 0 {
            return Ok(());
        }
        let auto_review = serde_json::to_string(&crate::model::AutoReview::default())?;
        connection.execute(sql!("attention/insert_upgrade_done"), params![flag, auto_review])?;
        Ok(())
    }

    /// Writes the encrypted attention row and its relay receipt in one transaction so a crash
    /// cannot leave a seen-id without its row (the applied-blob ring alone is not enough: it
    /// keeps only the newest ids).
    pub fn put_attention_with_applied(&self, id: &str, ciphertext: &[u8], blob_id: &str) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        tx.execute(sql!("attention/put"), params![id, ciphertext])?;
        append_applied_blob_tx(&tx, blob_id)?;
        tx.commit()?;
        Ok(())
    }

    /// Records that a relay blob id was fully applied when the row was already present
    /// (stale/equal revision). Prefer [`Self::put_attention_with_applied`] for new rows.
    pub fn mark_blob_applied(&self, blob_id: &str) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        append_applied_blob_tx(&tx, blob_id)?;
        tx.commit()?;
        Ok(())
    }

    /// Local content and its encrypted relay write commit together, so a crash cannot leave a
    /// successfully saved attention update without a durable upload.
    pub fn queue_attention(&self, id: &str, ciphertext: &[u8], item: &OutboxItem) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        tx.execute(sql!("attention/put"), params![id, ciphertext])?;
        queue_outbox_tx(&tx, item)?;
        tx.commit()?;
        Ok(())
    }

    #[cfg(test)]
    pub fn delete_metadata_for_tests(&self) -> anyhow::Result<()> {
        self.connection.lock().unwrap().execute("DELETE FROM metadata", [])?;
        Ok(())
    }
}
