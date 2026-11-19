-- ?1 message id, ?2 the version the work read: done, unless what it read changed meanwhile.
UPDATE message_search SET describe_pending = 0 WHERE message_id = ?1 AND image_version = ?2 AND describe_pending = 1 AND stale = 0
