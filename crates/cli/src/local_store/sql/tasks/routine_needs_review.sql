SELECT EXISTS(SELECT 1 FROM local_tasks WHERE account_epoch=?1 AND routine_id=?2 AND state IN ('interrupted','needs_review') AND resolved_at IS NULL)
