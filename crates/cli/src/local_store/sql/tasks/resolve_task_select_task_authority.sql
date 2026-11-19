SELECT EXISTS(SELECT 1 FROM task_authority WHERE account_epoch=?1 AND closed=0)
