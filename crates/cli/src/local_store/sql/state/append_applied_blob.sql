INSERT INTO applied_blobs (id, position) VALUES (?1, ?2)
ON CONFLICT(id) DO UPDATE SET position = excluded.position
