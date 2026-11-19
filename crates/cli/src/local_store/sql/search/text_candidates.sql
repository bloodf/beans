-- ?1 a JSON array of folded terms: messages whose folded text contains every one, in result
-- order (the `message_search_order` index, so a common term stops at the caller's limit).
SELECT s.message_id FROM message_search s
WHERE s.stale = 0 AND NOT EXISTS (SELECT 1 FROM json_each(?1) t WHERE instr(s.text, t.value) = 0)
ORDER BY s.created_at DESC, s.chat_id, s.position DESC, s.message_id
