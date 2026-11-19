DELETE FROM applied_blobs
WHERE id NOT IN (SELECT id FROM applied_blobs ORDER BY position DESC LIMIT 2000)
