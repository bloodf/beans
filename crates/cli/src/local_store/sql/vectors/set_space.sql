INSERT INTO message_vector_spaces(role, space) VALUES (?1, ?2)
ON CONFLICT(role) DO UPDATE SET space = excluded.space
