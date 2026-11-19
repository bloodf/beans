-- ?1 a candidate vector: its message and chunk, if the vector is still stored and its message is
-- current and not waiting to be embedded again for that vector's role.
SELECT m.message_json, v.chunk FROM message_vectors v
JOIN message_search s ON s.message_id = v.message_id
JOIN messages m ON m.id = v.message_id
WHERE v.id = ?1 AND s.stale = 0
  AND CASE v.role WHEN 'text' THEN s.embed_pending ELSE s.image_pending END = 0
  AND m.body_kind IN ('text', 'handoff', 'notice', 'permission')
