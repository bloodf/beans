-- A new text space: every message with text or file names waits for it.
UPDATE message_search SET embed_pending = 1 WHERE embed_pending = 0 AND (text != '' OR files != '')
