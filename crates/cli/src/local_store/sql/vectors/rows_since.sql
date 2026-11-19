SELECT id, message_id, chunk, scale, vector FROM message_vectors WHERE id > ?1 AND space = ?2 ORDER BY id
