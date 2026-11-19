SELECT COUNT(*) FROM messages
WHERE chat_id = ?1 AND position > ?2 AND is_complete = 1
  AND body_kind IN ('text', 'handoff')
