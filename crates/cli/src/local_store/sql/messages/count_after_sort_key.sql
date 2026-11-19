SELECT COUNT(*) FROM messages
WHERE chat_id = ?1 AND (sort_at > ?2 OR (sort_at = ?2 AND position > ?3))
