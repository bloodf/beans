INSERT INTO memory_fences(bot_id,json) VALUES(?1,?2) ON CONFLICT(bot_id) DO UPDATE SET json=excluded.json
