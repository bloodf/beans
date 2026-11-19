SELECT id, kind, recipient, ciphertext, slot_name, slot_keep_first, group_name
FROM outbox ORDER BY position DESC LIMIT 1
