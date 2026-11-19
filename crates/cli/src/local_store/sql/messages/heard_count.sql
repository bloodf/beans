SELECT COUNT(*) FROM messages
WHERE chat_id = ?1 AND is_complete = 1 AND body_kind IN ('text', 'handoff')
  AND NOT (author_kind = 'bot' AND author_bot_id = ?2)
