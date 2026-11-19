INSERT INTO messages (
    id, chat_id, position, sort_at, created_at, author_kind, author_bot_id,
    body_kind, is_complete, text_nonempty, message_json
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
ON CONFLICT(id) DO UPDATE SET chat_id = excluded.chat_id, position = excluded.position
