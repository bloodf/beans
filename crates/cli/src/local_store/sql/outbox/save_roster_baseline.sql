INSERT INTO roster_baseline (name, seq, json) VALUES (?1, ?2, ?3)
ON CONFLICT(name) DO UPDATE SET seq = excluded.seq, json = excluded.json
WHERE excluded.seq >= roster_baseline.seq
