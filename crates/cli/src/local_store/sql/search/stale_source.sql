-- A stale row's message JSON; NULL for a tool row, which is never decoded. No row: the
-- message is gone.
SELECT CASE WHEN body_kind = 'tool' THEN NULL ELSE message_json END FROM messages WHERE id = ?1
