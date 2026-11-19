INSERT INTO device_seen (id, seen_at) VALUES (?1, ?2)
ON CONFLICT(id) DO UPDATE SET seen_at = excluded.seen_at
