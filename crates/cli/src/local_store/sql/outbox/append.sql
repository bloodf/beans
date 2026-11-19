INSERT INTO outbox (id, kind, recipient, ciphertext, slot_name, slot_keep_first, group_name)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
