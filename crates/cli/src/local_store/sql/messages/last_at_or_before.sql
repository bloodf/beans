SELECT id FROM messages WHERE chat_id = ?1 AND sort_at * 1000.0 <= ?2
ORDER BY sort_at DESC, position DESC LIMIT 1
