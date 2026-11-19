INSERT INTO memory_runner_bindings(key,json) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET json=excluded.json
