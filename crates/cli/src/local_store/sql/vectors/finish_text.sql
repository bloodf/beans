-- ?1 message id, ?2 the version the work read: done, unless what it read changed meanwhile.
UPDATE message_search SET embed_pending = 0 WHERE message_id = ?1 AND embed_version = ?2 AND embed_pending = 1 AND stale = 0
