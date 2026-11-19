SELECT COALESCE(MAX(position),0)+1 FROM messages WHERE chat_id=?1
