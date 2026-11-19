SELECT EXISTS(SELECT 1 FROM local_tasks WHERE account_epoch=?1 AND task_id=?2 AND owner_epoch=?3 AND execution_id=?4 AND state='running' AND cancel_requested=0)
