SELECT message_json FROM (
    SELECT message_json, sort_at, position FROM messages
    WHERE chat_id = ?1 ORDER BY sort_at DESC, position DESC LIMIT ?2
) ORDER BY sort_at, position
