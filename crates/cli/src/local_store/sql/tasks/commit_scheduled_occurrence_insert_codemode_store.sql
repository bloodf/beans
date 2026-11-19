INSERT INTO codemode_store VALUES(?1,?2,?3,?4) ON CONFLICT(chat_id,bot_id,key) DO UPDATE SET json=excluded.json
