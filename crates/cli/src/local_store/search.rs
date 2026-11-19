//! Exact search: the folded `message_search` projection of every message, and its matcher.
//!
//! Each message has one narrow row holding its searchable fields folded exactly as
//! `search_matches` folds them, so SQL `instr` containment of a lowercase term is the same test
//! the in-memory matcher makes. Triggers (`sql/schema/search.sql`) keep its order columns in
//! step with `messages`, delete it with its message, and mark it stale when the message's JSON
//! changes, from this binary or any other. Stale rows are derived again before a search reads.

use super::*;

/// Bump when `Projection` or the derived tables change: they are dropped and rebuilt, and every
/// message derived again lazily.
const SEARCH_INDEX_VERSION: i64 = 2;

/// Creates or rebuilds the derived search tables, their triggers, and the upgrade seeding, in
/// one transaction: a store has a row, fresh or stale, for every message, or is unchanged.
pub(super) fn install_search_index(connection: &mut Connection) -> anyhow::Result<()> {
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let version: Option<i64> = tx.query_row(sql!("search/version"), [], |row| row.get(0)).optional()?;
    let current = version == Some(SEARCH_INDEX_VERSION);
    if !current {
        tx.execute_batch(sql!("schema/search_drop"))?;
    }
    tx.execute_batch(sql!("schema/search"))?;
    if !current {
        // Cheap stale rows now; each is decoded once, on the next search or indexing pass.
        tx.execute(sql!("search/seed"), [])?;
        tx.execute(sql!("search/set_version"), [SEARCH_INDEX_VERSION])?;
    }
    tx.commit()?;
    Ok(())
}

/// Derives one batch of stale rows: the upgrade backfill, a projection change, or messages
/// another binary wrote. Only those rows are decoded, and tool rows never are. Returns whether
/// rows may remain. Each batch commits on its own, so an interrupted backfill resumes.
fn refresh_batch(connection: &mut Connection) -> anyhow::Result<bool> {
    // The common case is a plain read: a search takes no write lock unless rows are stale.
    if !connection.query_row(sql!("search/any_stale"), [], |row| row.get::<_, bool>(0))? {
        return Ok(false);
    }
    {
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ids: Vec<String> = tx.prepare(sql!("search/stale_batch"))?.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
        if ids.is_empty() {
            return Ok(false);
        }
        {
            let mut load = tx.prepare(sql!("search/stale_source"))?;
            let mut store = tx.prepare(sql!("search/store_projection"))?;
            for id in ids {
                match load.query_row([&id], |row| row.get::<_, Option<String>>(0)).optional()? {
                    None => {
                        tx.execute(sql!("search/forget"), [&id])?;
                    }
                    Some(json) => {
                        let projection = match json {
                            Some(json) => Projection::of(&serde_json::from_str::<Message>(&json).context("decoding search result")?.body),
                            None => Projection::default(), // A tool row: nothing to search.
                        };
                        store.execute(params![id, projection.text, projection.files, projection.images])?;
                    }
                }
            }
        }
        tx.commit()?;
    }
    Ok(true)
}

impl LocalStore {
    /// The newest `limit` messages' folded search text, and whether any message has pictures:
    /// what a model suggestion reads of the chats. Nothing is derived for it.
    pub(crate) fn chat_sample(&self, limit: usize) -> anyhow::Result<(Vec<String>, bool)> {
        let connection = self.connection.lock().unwrap();
        let texts = connection.prepare_cached(sql!("search/recent_texts"))?.query_map([limit as i64], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
        let pictures = connection.query_row(sql!("search/has_pictures"), [], |row| row.get(0))?;
        Ok((texts, pictures))
    }

    /// Derives every stale row, one batch per lock: an upgrade's backfill never holds the store
    /// for longer than one batch, so turns and sync go on while it runs.
    pub(crate) fn refresh_search(&self) -> anyhow::Result<()> {
        while refresh_batch(&mut self.connection.lock().unwrap())? {}
        Ok(())
    }
}

/// Derives a row from exactly the JSON stored for it. A write that kept older JSON (an older
/// page meeting a row already here) leaves the row as its triggers left it.
pub(super) fn index_message_tx(tx: &Transaction<'_>, message: &Message, json: &str) -> anyhow::Result<()> {
    let projection = Projection::of(&message.body);
    tx.execute(sql!("search/index_message"), params![message.id, projection.text, projection.files, projection.images, json])?;
    Ok(())
}

/// What search derives from a message body.
#[derive(Default)]
struct Projection {
    /// The search text, folded.
    text: String,
    /// Attachment names, each folded, one per line: no term spans two names.
    files: String,
    /// Image attachment ids, one per line, for semantic image search.
    images: String,
}

impl Projection {
    fn of(body: &Body) -> Projection {
        let text = search_body_text(body).map(str::to_lowercase).unwrap_or_default();
        let Body::Text { attachments, .. } = body else { return Projection { text, ..Default::default() } };
        let lines = |parts: Vec<String>| parts.join("\n");
        Projection {
            text,
            files: lines(attachments.iter().map(|a| a.name.to_lowercase()).collect()),
            images: lines(attachments.iter().filter(|a| a.is_image()).map(|a| a.id.clone()).collect()),
        }
    }
}

/// The one field of a body that message search matches and snippets.
pub(crate) fn search_body_text(body: &Body) -> Option<&str> {
    match body {
        Body::Text { text, .. } => Some(text),
        Body::Handoff { reason, .. } => Some(reason),
        Body::Notice { text, .. } => Some(text),
        Body::Permission { summary, .. } => Some(summary),
        Body::Tool { .. } => None,
    }
}

/// Which folded column must contain every term.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchScope {
    /// The body's search text.
    Text,
    /// The attachment names: a superset, since terms may sit in different names.
    Files,
}

pub(crate) fn search_terms(input: &str) -> Vec<String> {
    input.split(|character: char| !character.is_alphanumeric()).filter(|term| !term.is_empty()).map(str::to_lowercase).collect()
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
    format!("{}{}{}", if start > 0 { "… " } else { "" }, words[start..end].join(" "), if end < words.len() { " …" } else { "" })
}

impl LocalStore {
    /// Visits stored messages whose folded `scope` contains every term, newest first
    /// (`created_at DESC, chat_id, position DESC, id`), until `visit` returns false. The SQL
    /// prefilter never misses a true hit; callers confirm each candidate with `search_matches`.
    /// Rows outside the prefilter are neither read nor decoded.
    pub(crate) fn visit_search_candidates(&self, terms: &[String], scope: SearchScope, mut visit: impl FnMut(Message) -> bool) -> anyhow::Result<()> {
        self.refresh_search()?;
        let connection = self.connection.lock().unwrap();
        let sql = match scope {
            SearchScope::Text => sql!("search/text_candidates"),
            SearchScope::Files => sql!("search/file_candidates"),
        };
        let mut candidates = connection.prepare_cached(sql)?;
        let mut load = connection.prepare_cached(sql!("search/candidate_message"))?;
        // Terms travel as one JSON array, so one cached statement serves any number of them.
        let mut rows = candidates.query([serde_json::to_string(terms)?])?;
        while let Some(row) = rows.next()? {
            let Some(json) = load.query_row([row.get::<_, String>(0)?], |row| row.get::<_, String>(0)).optional()? else {
                continue;
            };
            if !visit(serde_json::from_str(&json).context("decoding search result")?) {
                break;
            }
        }
        Ok(())
    }
}
