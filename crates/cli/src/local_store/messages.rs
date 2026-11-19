//! The transcript: messages in chat order, pages, model context, and the rows turns look up.

use super::*;

pub struct Upsert {
    pub previous: Option<Message>,
    pub changed: bool,
}

pub(super) fn upsert_message_tx(tx: &Transaction<'_>, message: &Message) -> anyhow::Result<Upsert> {
    let json = serde_json::to_string(message)?;
    let existing: Option<(String, String, i64)> = tx.query_row(sql!("messages/stored_for_upsert"), [&message.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let previous = existing.as_ref().map(|(stored,_,_)|serde_json::from_str(stored)).transpose()?;
    if existing.as_ref().is_some_and(|(stored,_,_)|stored==&json) { return Ok(Upsert { previous, changed:false }); }
    let position = match existing.as_ref() {
        Some((_,chat,position)) if chat==&message.chat_id => *position,
        _ => tx.query_row(sql!("messages/next_position"),[&message.chat_id],|r|r.get(0))?,
    };
    let (author_kind,author_bot_id)=author_columns(&message.author);
    tx.execute(sql!("messages/upsert"),
        params![message.id,message.chat_id,position,message.promoted_at.unwrap_or(message.created_at),message.created_at,author_kind,author_bot_id,body_kind(&message.body),message.is_complete(),matches!(&message.body,Body::Text{text,..} if !text.trim().is_empty()),json])?;
    index_message_tx(tx,message,&json)?;
    Ok(Upsert {previous,changed:true})
}

pub(super) fn collect_messages(
    rows: impl Iterator<Item = rusqlite::Result<String>>,
) -> anyhow::Result<Vec<Message>> {
    rows.map(|row| {
        let json = row?;
        serde_json::from_str(&json).context("decoding stored message")
    })
    .collect()
}

pub(super) fn author_columns(author: &Author) -> (&'static str, Option<&str>) {
    match author {
        Author::You => ("you", None),
        Author::Bot { bot_id } => ("bot", Some(bot_id)),
        Author::System => ("system", None),
    }
}

pub(super) fn body_kind(body: &Body) -> &'static str {
    match body {
        Body::Text { .. } => "text",
        Body::Tool { .. } => "tool",
        Body::Handoff { .. } => "handoff",
        Body::Notice { .. } => "notice",
        Body::Permission { .. } => "permission",
    }
}

impl LocalStore {
    pub fn upsert(&self, message: &Message) -> anyhow::Result<Upsert> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        let result = upsert_message_tx(&tx, message)?;
        tx.commit()?;
        Ok(result)
    }

    /// Where this Device's copy of the chat begins in the relay's log, when the relay has
    /// older messages than the ones here. `None` for a chat that is here whole.
    pub fn history_before(&self, chat_id: &str) -> anyhow::Result<Option<i64>> {
        let connection = self.connection.lock().unwrap();
        connection
            .query_row(sql!("messages/history_before"), [chat_id], |row| row.get(0))
            .optional()
            .map_err(Into::into)
    }

    /// `Some(place)`: the relay has messages placed below it that are not here. `None`: the
    /// chat is here whole.
    pub fn set_history_before(&self, chat_id: &str, before: Option<i64>) -> anyhow::Result<()> {
        let connection = self.connection.lock().unwrap();
        match before {
            Some(place) => connection.execute(
                sql!("messages/set_history_before"),
                params![chat_id, place],
            )?,
            None => connection.execute(sql!("messages/forget_history_before"), [chat_id])?,
        };
        Ok(())
    }

    /// Puts messages read backwards from the relay ahead of everything the chat has here.
    /// `newest_first` is the page in that order, so each lands before the one after it. One
    /// that is here already (a late edit arrived through the log and went to the end) moves
    /// to its place.
    pub fn insert_older(&self, newest_first: &[Message]) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        for message in newest_first {
            let position: i64 = tx.query_row(
                sql!("messages/position_before_first"),
                [&message.chat_id],
                |row| row.get(0),
            )?;
            let (author_kind, author_bot_id) = author_columns(&message.author);
            let text_nonempty = matches!(&message.body, Body::Text { text, .. } if !text.trim().is_empty());
            let json = serde_json::to_string(message)?;
            tx.execute(
                sql!("messages/insert_older"),
                params![
                    message.id,
                    message.chat_id,
                    position,
                    message.promoted_at.unwrap_or(message.created_at),
                    message.created_at,
                    author_kind,
                    author_bot_id,
                    body_kind(&message.body),
                    message.is_complete(),
                    text_nonempty,
                    json,
                ],
            )?;
            index_message_tx(&tx, message, &json)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn message(&self, chat_id: &str, message_id: &str) -> anyhow::Result<Option<Message>> {
        let connection = self.connection.lock().unwrap();
        let json: Option<String> = connection
            .query_row(
                sql!("messages/message"),
                params![chat_id, message_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).context("decoding stored message"))
            .transpose()
    }

    /// Every message of a chat in its order.
    pub fn all(&self, chat_id: &str) -> anyhow::Result<Vec<Message>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection
            .prepare(sql!("messages/all"))?;
        let rows = statement.query_map([chat_id], |row| row.get::<_, String>(0))?;
        collect_messages(rows)
    }

    /// The model-visible ordering, with a valid compaction cursor taking precedence over the
    /// fallback limit. Without a valid cursor only the newest `limit` rows are materialized.
    pub fn context(
        &self,
        chat_id: &str,
        after: Option<&str>,
        limit: Option<usize>,
    ) -> anyhow::Result<(Vec<Message>, bool)> {
        let connection = self.connection.lock().unwrap();
        let cursor: Option<(f64, i64)> = match after {
            Some(id) => connection
                .query_row(
                    sql!("messages/sort_key"),
                    params![chat_id, id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?,
            None => None,
        };
        let (mut messages, cursor_found) = if let Some((sort_at, position)) = cursor {
            let mut statement = connection.prepare(
                sql!("messages/context_after"),
            )?;
            let rows = statement.query_map(params![chat_id, sort_at, position], |row| {
                row.get::<_, String>(0)
            })?;
            let messages = collect_messages(rows)?;
            (messages, true)
        } else if let Some(limit) = limit {
            let mut statement = connection.prepare(
                sql!("messages/context_newest"),
            )?;
            let rows = statement.query_map(params![chat_id, limit as i64], |row| {
                row.get::<_, String>(0)
            })?;
            let messages = collect_messages(rows)?;
            (messages, false)
        } else {
            let mut statement = connection.prepare(
                sql!("messages/context_all"),
            )?;
            let rows = statement.query_map([chat_id], |row| row.get::<_, String>(0))?;
            let messages = collect_messages(rows)?;
            (messages, false)
        };
        // Defend against an old malformed row without changing the ordering of healthy rows.
        messages.retain(|message| message.chat_id == chat_id);
        Ok((messages, cursor_found))
    }

    /// A page in insertion order, oldest first. `before` is exclusive; an unknown cursor means
    /// the end of the chat, matching the previous in-memory API.
    pub fn page(
        &self,
        chat_id: &str,
        before: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<(Vec<Message>, bool)> {
        let connection = self.connection.lock().unwrap();
        let end = match before {
            Some(id) => connection
                .query_row(
                    sql!("messages/position_of"),
                    params![chat_id, id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .unwrap_or(i64::MAX),
            None => i64::MAX,
        };
        let mut statement = connection.prepare(
            sql!("messages/page_before"),
        )?;
        let mut messages = collect_messages(
            statement.query_map(params![chat_id, end, (limit + 1) as i64], |row| {
                row.get::<_, String>(0)
            })?,
        )?;
        let has_more = messages.len() > limit;
        if has_more {
            messages.pop();
        }
        messages.reverse();
        Ok((messages, has_more))
    }

    /// The newest `limit` messages after `message_id`, oldest first, and how many of the ones
    /// between it and them were left out.
    pub fn newest_after(&self, chat_id: &str, message_id: &str, limit: usize) -> anyhow::Result<(Vec<Message>, usize)> {
        let connection = self.connection.lock().unwrap();
        let Some(position) = connection
            .query_row(sql!("messages/position_of"), params![chat_id, message_id], |row| row.get::<_, i64>(0))
            .optional()?
        else {
            return Ok((Vec::new(), 0));
        };
        let total: i64 =
            connection.query_row(sql!("messages/count_after_position"), params![chat_id, position], |row| row.get(0))?;
        let mut statement =
            connection.prepare(sql!("messages/newest_after_position"))?;
        let mut messages = collect_messages(statement.query_map(params![chat_id, position, limit as i64], |row| row.get::<_, String>(0))?)?;
        messages.reverse();
        let left_out = (total as usize).saturating_sub(messages.len());
        Ok((messages, left_out))
    }

    pub fn messages_after(&self, chat_id: &str, message_id: &str) -> anyhow::Result<Vec<Message>> {
        let connection = self.connection.lock().unwrap();
        let Some(position) = connection
            .query_row(
                sql!("messages/position_of"),
                params![chat_id, message_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        else {
            return Ok(Vec::new());
        };
        let mut statement = connection.prepare(
            sql!("messages/after_position"),
        )?;
        let rows =
            statement.query_map(params![chat_id, position], |row| row.get::<_, String>(0))?;
        collect_messages(rows)
    }

    /// Every `bash` card (`Body::Tool.run`), in any chat: for the ones a Beans that quit left
    /// open.
    pub fn command_rows(&self) -> anyhow::Result<Vec<Message>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(sql!("messages/command_rows"))?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        collect_messages(rows)
    }

    /// Proposal cards persisted across Runner restarts; the caller checks their decision and waiter.
    pub fn proposal_cards(&self) -> anyhow::Result<Vec<Message>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            sql!("messages/proposal_cards"),
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        Ok(collect_messages(rows)?.into_iter().filter(|message| matches!(&message.body, Body::Permission { tool, .. } if tool == "propose")).collect())
    }

    /// A plugin's sign-in cards (`Body::Permission` with `tool` `connect`) in one chat, or in
    /// every chat, in order, each with whether it came after the user last wrote in its chat.
    pub fn sign_in_cards(&self, chat_id: Option<&str>, plugin_id: &str) -> anyhow::Result<Vec<(Message, bool)>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            sql!("messages/sign_in_cards"),
        )?;
        let rows = statement.query_map([chat_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)))?;
        let mut cards = Vec::new();
        for row in rows {
            let (json, after_user) = row?;
            let message: Message = serde_json::from_str(&json).context("decoding stored message")?;
            if matches!(&message.body, Body::Permission { plugin_id: p, tool, .. } if p == plugin_id && tool == "connect") {
                cards.push((message, after_user));
            }
        }
        Ok(cards)
    }

    pub fn count_after(&self, chat_id: &str, message_id: Option<&str>) -> anyhow::Result<usize> {
        let connection = self.connection.lock().unwrap();
        let count = match message_id {
            Some(id) => {
                let cursor: Option<(f64, i64)> = connection
                    .query_row(
                        sql!("messages/sort_key"),
                        params![chat_id, id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                match cursor {
                    Some((sort_at, position)) => connection.query_row(
                        sql!("messages/count_after_sort_key"),
                        params![chat_id, sort_at, position],
                        |row| row.get::<_, i64>(0),
                    )?,
                    None => connection.query_row(
                        sql!("messages/count_in_chat"),
                        [chat_id],
                        |row| row.get::<_, i64>(0),
                    )?,
                }
            }
            None => connection.query_row(
                sql!("messages/count_in_chat"),
                [chat_id],
                |row| row.get::<_, i64>(0),
            )?,
        };
        Ok(count as usize)
    }

    pub fn last_at_or_before(
        &self,
        chat_id: &str,
        timestamp_ms: u64,
    ) -> anyhow::Result<Option<String>> {
        let connection = self.connection.lock().unwrap();
        connection
            .query_row(
                sql!("messages/last_at_or_before"),
                params![chat_id, timestamp_ms as f64],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn last_user_at(&self) -> anyhow::Result<Option<i64>> {
        let connection = self.connection.lock().unwrap();
        connection
            .query_row(
                sql!("messages/last_user_at"),
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    /// What the turn `message_id` started acts on: that message, or the closest one before it
    /// that asks for work: the user's message, a teammate's handoff, or a routine's marker. A
    /// turn that a command's end started opens on the command's card, after the request it served.
    pub fn request_at(&self, chat_id: &str, message_id: &str) -> anyhow::Result<Option<Message>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            sql!("messages/request_at"),
        )?;
        let rows = statement.query_map(params![chat_id, message_id], |row| row.get::<_, String>(0))?;
        for json in rows {
            let message: Message = serde_json::from_str(&json?).context("decoding stored message")?;
            // Other notices only say what happened.
            if !matches!(message.body, Body::Notice { routine_id: None, .. }) {
                return Ok(Some(message));
            }
        }
        Ok(None)
    }

    /// The user's latest text in the chat, from `since` on.
    pub fn last_user_text(&self, chat_id: &str, since: &str) -> anyhow::Result<Option<String>> {
        let connection = self.connection.lock().unwrap();
        let json: Option<String> = connection
            .query_row(
                sql!("messages/last_user_text"),
                params![chat_id, since],
                |row| row.get(0),
            )
            .optional()?;
        Ok(json
            .map(|json| serde_json::from_str::<Message>(&json).context("decoding stored message"))
            .transpose()?
            .and_then(|message| match message.body {
                Body::Text { text, .. } if !text.trim().is_empty() => Some(text.trim().to_string()),
                _ => None,
            }))
    }

    pub fn last_bot_text(&self, chat_id: &str, bot_id: &str) -> anyhow::Result<Option<Message>> {
        let connection = self.connection.lock().unwrap();
        let json: Option<String> = connection
            .query_row(
                sql!("messages/last_bot_text"),
                params![chat_id, bot_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).context("decoding stored message"))
            .transpose()
    }

    pub fn text_messages(
        &self,
        chat_id: &str,
        since: Option<i64>,
        until: Option<i64>,
    ) -> anyhow::Result<Vec<Message>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            sql!("messages/text_messages"),
        )?;
        let rows = statement.query_map(params![chat_id, since, until], |row| {
            row.get::<_, String>(0)
        })?;
        collect_messages(rows)
    }

    pub fn heard_count(&self, chat_id: &str, bot_id: &str) -> anyhow::Result<usize> {
        let connection = self.connection.lock().unwrap();
        let count = connection.query_row(
            sql!("messages/heard_count"),
            params![chat_id, bot_id],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count as usize)
    }

    pub fn new_count_since_bot_spoke(&self, chat_id: &str, bot_id: &str) -> anyhow::Result<usize> {
        let connection = self.connection.lock().unwrap();
        let last: Option<i64> = connection
            .query_row(
                sql!("messages/last_bot_position"),
                params![chat_id, bot_id],
                |row| row.get(0),
            )
            .optional()?;
        let count = connection.query_row(
            sql!("messages/count_heard_after_position"),
            params![chat_id, last.unwrap_or(0)],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count as usize)
    }

    pub fn remove(&self, chat_id: &str, message_id: &str) -> anyhow::Result<bool> {
        let connection = self.connection.lock().unwrap();
        let removed = connection.execute(
            sql!("messages/remove"),
            params![chat_id, message_id],
        )? > 0;
        Ok(removed)
    }

    /// Remove rows whose chat metadata no longer exists. The chat table is authoritative;
    /// this catches interrupted or concurrent cleanup before the app serves a snapshot.
    pub fn retain_chats(&self, chat_ids: &[String]) -> anyhow::Result<()> {
        let valid: std::collections::HashSet<&str> = chat_ids.iter().map(String::as_str).collect();
        let valid_groups: std::collections::HashSet<String> = chat_ids
            .iter()
            .map(|id| crate::model::relay_name(id))
            .collect();
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        let stored_chats: Vec<String> = {
            let mut statement = tx.prepare(sql!("messages/stored_chats"))?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for chat_id in stored_chats {
            if !valid.contains(chat_id.as_str()) {
                tx.execute(sql!("messages/delete_chat"), [&chat_id])?;
            }
        }
        let scripted_chats: Vec<String> = {
            let mut statement = tx.prepare(sql!("messages/scripted_chats"))?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for chat_id in scripted_chats {
            if !valid.contains(chat_id.as_str()) {
                tx.execute(sql!("messages/delete_chat_codemode"), [&chat_id])?;
            }
        }
        let queued_groups: Vec<String> = {
            let mut statement =
                tx.prepare(sql!("messages/queued_groups"))?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for group in queued_groups {
            if !valid_groups.contains(&group) {
                tx.execute(sql!("messages/delete_group_outbox"), [&group])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
