SELECT message_json FROM messages
WHERE chat_id = ?1 AND body_kind = 'text' AND is_complete = 1
  AND (?2 IS NULL OR created_at >= ?2)
  AND (?3 IS NULL OR created_at <= ?3)
ORDER BY position
