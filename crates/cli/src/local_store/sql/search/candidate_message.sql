SELECT message_json FROM messages WHERE id = ?1 AND body_kind IN ('text', 'handoff', 'notice', 'permission')
