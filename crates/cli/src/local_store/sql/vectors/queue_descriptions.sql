-- ?1 limit: finished (complete or failed) messages waiting in this queue, newest first, read
-- straight from `message_search_describe_queue`. A reply still streaming waits for its last words.
SELECT s.message_id, s.image_version, m.message_json FROM message_search s JOIN messages m ON m.id = s.message_id
WHERE s.describe_pending = 1 AND s.stale = 0 AND m.body_kind IN ('text', 'handoff', 'notice', 'permission')
  AND (m.is_complete = 1 OR json_extract(m.message_json, '$.state.kind') = 'failed')
ORDER BY s.created_at DESC, s.message_id LIMIT ?1
