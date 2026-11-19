UPDATE local_tasks SET state='running' WHERE account_epoch=?1 AND task_id=?2 AND owner_epoch=?3 AND execution_id=?4 AND state='queued' AND cancel_requested=0
