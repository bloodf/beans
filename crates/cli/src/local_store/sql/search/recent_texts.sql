-- ?1 how many: the newest messages' folded search text, for guessing which languages the chats
-- are in. Rows not derived yet are simply not counted.
SELECT text FROM message_search WHERE stale = 0 AND text != '' ORDER BY created_at DESC LIMIT ?1
