-- ?1 role: messages with vectors of that role, and messages waiting for them.
SELECT (SELECT count(DISTINCT message_id) FROM message_vectors WHERE role = ?1),
       (SELECT count(*) FROM message_search WHERE CASE ?1 WHEN 'text' THEN embed_pending ELSE image_pending END = 1)
