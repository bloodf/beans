INSERT OR IGNORE INTO task_fences(account_epoch,task_id) SELECT ?1,?2 WHERE EXISTS(SELECT 1 FROM task_authority WHERE account_epoch=?1 AND closed=0)
