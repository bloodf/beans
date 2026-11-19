-- ?1 message id, ?2 text, ?3 files, ?4 images, derived from ?5, the JSON just written: a row
-- whose stored JSON differs keeps what its triggers left.
INSERT INTO message_search(message_id, chat_id, created_at, position, stale, text, files, images)
SELECT id, chat_id, created_at, position, 0, ?2, ?3, ?4 FROM messages WHERE id = ?1 AND message_json = ?5
ON CONFLICT(message_id) DO UPDATE SET
    chat_id = excluded.chat_id, created_at = excluded.created_at, position = excluded.position,
    stale = 0, text = excluded.text, files = excluded.files, images = excluded.images
