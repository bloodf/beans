-- ?1 flag, ?2 default Auto-review JSON: no settings row yet (identity/save_state has not landed).
-- A minimal row keeps the upgrade marker and still loads (Auto-review needs its fields), and a
-- later save_state fills the other columns.
INSERT INTO metadata (id, auto_review_json, last_seq, roster_slot_seq, machine_blob_hash, credentials_uploaded, paused, policy_json, attention_upgrade)
VALUES (1, ?2, 0, 0, NULL, 0, 0, '{}', ?1)
ON CONFLICT(id) DO UPDATE SET attention_upgrade = excluded.attention_upgrade
