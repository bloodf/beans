-- One stale row per message; each is derived on first use.
INSERT INTO message_search(message_id, chat_id, created_at, position)
SELECT id, chat_id, created_at, position FROM messages
