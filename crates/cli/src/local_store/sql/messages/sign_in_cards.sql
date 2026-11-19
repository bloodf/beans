SELECT m.message_json, m.position > COALESCE(
    (SELECT u.position FROM messages u WHERE u.chat_id = m.chat_id AND u.author_kind = 'you' ORDER BY u.position DESC LIMIT 1),
    m.position - 1)
FROM messages m
WHERE (?1 IS NULL OR m.chat_id = ?1) AND m.body_kind = 'permission' AND m.message_json LIKE '%"tool":"connect"%'
ORDER BY m.chat_id, m.position
