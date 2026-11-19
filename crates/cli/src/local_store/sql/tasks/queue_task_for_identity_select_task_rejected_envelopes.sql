SELECT EXISTS(SELECT 1 FROM task_rejected_envelopes WHERE account_identity=?1 AND task_id=?2)
