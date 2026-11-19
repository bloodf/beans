SELECT message_json FROM messages
WHERE chat_id = ?1 AND author_kind = 'bot' AND author_bot_id = ?2
  AND body_kind = 'text' AND is_complete = 1
ORDER BY position DESC LIMIT 1
