SELECT message_json FROM messages
WHERE chat_id = ?1 AND author_kind = 'you' AND body_kind = 'text' AND text_nonempty = 1
  AND position >= (SELECT position FROM messages WHERE chat_id = ?1 AND id = ?2)
ORDER BY position DESC LIMIT 1
