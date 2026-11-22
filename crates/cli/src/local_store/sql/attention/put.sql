-- ?1 id, ?2 ciphertext: attention contents stay encrypted at rest.
INSERT INTO attention (id, ciphertext) VALUES (?1, ?2)
ON CONFLICT(id) DO UPDATE SET ciphertext = excluded.ciphertext
