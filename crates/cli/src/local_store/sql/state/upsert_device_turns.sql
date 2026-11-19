INSERT INTO device_turns (id, json) VALUES (?1, ?2)
ON CONFLICT(id) DO UPDATE SET json = excluded.json
