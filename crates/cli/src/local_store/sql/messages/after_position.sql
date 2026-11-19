SELECT message_json FROM messages WHERE chat_id = ?1 AND position > ?2 ORDER BY position
