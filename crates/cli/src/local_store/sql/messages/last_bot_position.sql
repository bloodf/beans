SELECT position FROM messages
WHERE chat_id = ?1 AND author_kind = 'bot' AND author_bot_id = ?2 AND body_kind = 'text'
ORDER BY position DESC LIMIT 1
