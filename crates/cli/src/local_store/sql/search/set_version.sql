INSERT INTO message_search_version(id, version) VALUES (1, ?1)
ON CONFLICT(id) DO UPDATE SET version = excluded.version
