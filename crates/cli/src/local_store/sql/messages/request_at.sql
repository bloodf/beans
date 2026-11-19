SELECT message_json FROM messages
WHERE chat_id = ?1
  AND position <= (SELECT position FROM messages WHERE chat_id = ?1 AND id = ?2)
  AND ((author_kind = 'you' AND body_kind = 'text')
       OR body_kind = 'handoff'
       OR (author_kind = 'system' AND body_kind = 'notice'))
ORDER BY position DESC
