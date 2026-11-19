SELECT EXISTS(SELECT 1 FROM task_authority WHERE id=1 AND account_epoch=?1 AND owner_epoch=?2 AND closed=0)
