INSERT INTO chat_history (chat_id, before_place) VALUES (?1, ?2)
ON CONFLICT(chat_id) DO UPDATE SET before_place = excluded.before_place
