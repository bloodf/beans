-- ?1 a JSON array of folded terms: messages whose attachment names hold every one (perhaps
-- in different names, which the caller checks), in result order.
SELECT s.message_id FROM message_search s
WHERE s.stale = 0 AND NOT EXISTS (SELECT 1 FROM json_each(?1) t WHERE instr(s.files, t.value) = 0)
ORDER BY s.created_at DESC, s.chat_id, s.position DESC, s.message_id
