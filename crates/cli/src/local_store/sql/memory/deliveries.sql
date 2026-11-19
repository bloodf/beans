SELECT json FROM memory_deliveries WHERE bot_id=?1 ORDER BY (state!='completed') DESC,rowid DESC LIMIT 1000
