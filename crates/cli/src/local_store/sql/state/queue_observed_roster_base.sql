INSERT OR IGNORE INTO roster_baseline (name, seq, json) SELECT 'queued', seq, json FROM roster_baseline WHERE name = 'observed'
