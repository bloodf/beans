INSERT INTO metadata (id, auto_review_json, last_seq, roster_slot_seq, machine_blob_hash, credentials_uploaded, paused, policy_json)
VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)
ON CONFLICT(id) DO UPDATE SET
    auto_review_json = excluded.auto_review_json,
    last_seq = excluded.last_seq,
    roster_slot_seq = excluded.roster_slot_seq,
    machine_blob_hash = excluded.machine_blob_hash,
    credentials_uploaded = excluded.credentials_uploaded,
    paused = excluded.paused,
    policy_json = excluded.policy_json
