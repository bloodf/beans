-- A new image space: every message with images waits for it.
UPDATE message_search SET image_pending = 1 WHERE image_pending = 0 AND images != ''
