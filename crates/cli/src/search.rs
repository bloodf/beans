//! Device-local search over current roster and canonical message rows. No files are opened.
//! Message rows are found through the folded `message_search` prefilter; only candidates
//! are decoded, and each is confirmed with the same matcher the roster search uses.
use std::collections::HashSet;

use serde_json::{json, Value};

use crate::{app::App, local_store::{search_body_text, search_matches, search_snippet, search_terms, SearchScope}, model::Body};

pub(crate) fn query(app: &App, query: &str, limit: usize) -> anyhow::Result<Value> {
    let terms = search_terms(query);
    if terms.is_empty() {
        return Ok(json!({"chats": [], "messages": [], "files": [], "history_complete": true}));
    }
    // Keep roster deletion outside the scan, so a removed chat cannot consume a result limit.
    let state = app.state.lock().unwrap();
    let mut chats = Vec::new();
    for chat in &state.chats {
        let mut parts = Vec::new();
        parts.extend(chat.meta.title.as_deref());
        parts.extend(chat.meta.description.as_deref());
        for id in &chat.meta.bot_ids {
            if let Some(bot) = state.bots.iter().find(|bot| bot.id == *id) {
                parts.extend([bot.name.as_str(), bot.description.as_str()]);
            }
        }
        let text = parts.join(" ");
        if search_matches(&text, &terms) {
            chats.push(json!({"chat_id": chat.meta.id, "snippet": search_snippet(&text, &terms)}));
            if chats.len() == limit { break; }
        }
    }
    let roster: HashSet<&str> = state.chats.iter().map(|chat| chat.meta.id.as_str()).collect();
    // Each category walks its own candidates, so a full one never keeps decoding for the other.
    let mut messages = Vec::new();
    app.store.visit_search_candidates(&terms, SearchScope::Text, |message| {
        if messages.len() >= limit { return false; }
        if !roster.contains(message.chat_id.as_str()) { return true; }
        let Some(text) = search_body_text(&message.body) else { return true };
        if search_matches(text, &terms) {
            messages.push(json!({
                "chat_id": message.chat_id, "message_id": message.id,
                "snippet": search_snippet(text, &terms), "author": message.author,
                "created_at": message.created_at,
            }));
        }
        messages.len() < limit
    })?;
    let mut files = Vec::new();
    app.store.visit_search_candidates(&terms, SearchScope::Files, |message| {
        if files.len() >= limit { return false; }
        if !roster.contains(message.chat_id.as_str()) { return true; }
        let Body::Text { attachments, .. } = &message.body else { return true };
        for attachment in attachments {
            if files.len() < limit && search_matches(&attachment.name, &terms) {
                files.push(json!({
                    "chat_id": message.chat_id, "message_id": message.id,
                    "attachment_id": attachment.id, "name": attachment.name,
                    "snippet": search_snippet(&attachment.name, &terms),
                    "created_at": message.created_at,
                }));
            }
        }
        files.len() < limit
    })?;
    let connection = app.store.connection.lock().unwrap();
    let mut history = connection.prepare("SELECT chat_id FROM chat_history")?;
    let cursors = history.query_map([], |row| row.get::<_, String>(0))?;
    let mut history_complete = true;
    for cursor in cursors {
        let id = cursor?;
        if roster.contains(id.as_str()) {
            history_complete = false;
            break;
        }
    }
    Ok(json!({"chats": chats, "messages": messages, "files": files, "history_complete": history_complete}))
}
