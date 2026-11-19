SELECT COALESCE(MIN(position), 1) - 1 FROM messages WHERE chat_id = ?1
