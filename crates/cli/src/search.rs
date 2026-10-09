//! Device-local search over current roster and canonical message rows. No files are opened.
use anyhow::Context;
use serde_json::{json, Value};

use crate::{app::App, local_store::{search_matches, search_snippet, search_terms}, model::{Body, Message}};

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
    let connection = app.store.connection.lock().unwrap();
    let mut statement = connection.prepare(
        "SELECT message_json FROM messages
         WHERE body_kind IN ('text', 'handoff', 'notice', 'permission')
         ORDER BY created_at DESC, chat_id, position DESC, id",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    let mut messages = Vec::new();
    let mut files = Vec::new();
    for row in rows {
        let message: Message = serde_json::from_str(&row?).context("decoding search result")?;
        if !state.chats.iter().any(|chat| chat.meta.id == message.chat_id) { continue; }
        let text = match &message.body {
            Body::Text { text, attachments, .. } => {
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
                text.as_str()
            }
            Body::Handoff { reason, .. } => reason,
            Body::Notice { text, .. } => text,
            Body::Permission { summary, .. } => summary,
            Body::Tool { .. } => continue,
        };
        if messages.len() < limit && search_matches(text, &terms) {
            messages.push(json!({
                "chat_id": message.chat_id, "message_id": message.id,
                "snippet": search_snippet(text, &terms), "author": message.author,
                "created_at": message.created_at,
            }));
        }
        if messages.len() == limit && files.len() == limit { break; }
    }
    let mut history = connection.prepare("SELECT chat_id FROM chat_history")?;
    let cursors = history.query_map([], |row| row.get::<_, String>(0))?;
    let mut history_complete = true;
    for cursor in cursors {
        let id = cursor?;
        if state.chats.iter().any(|chat| chat.meta.id == id) {
            history_complete = false;
            break;
        }
    }
    Ok(json!({"chats": chats, "messages": messages, "files": files, "history_complete": history_complete}))
}
