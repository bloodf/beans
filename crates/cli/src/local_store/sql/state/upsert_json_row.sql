INSERT INTO {table} (id, position, json) VALUES (?1, ?2, ?3)
ON CONFLICT(id) DO UPDATE SET position = excluded.position, json = excluded.json
