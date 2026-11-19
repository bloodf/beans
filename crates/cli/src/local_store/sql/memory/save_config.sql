INSERT INTO memory_config(id,ciphertext) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET ciphertext=excluded.ciphertext
