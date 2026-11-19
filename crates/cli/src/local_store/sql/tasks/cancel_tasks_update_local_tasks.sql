UPDATE local_tasks SET cancel_requested=1 WHERE account_epoch=?1 AND (?2 IS NULL OR task_id=?2) AND (?3 IS NULL OR chat_id=?3) AND (?4 IS NULL OR bot_id=?4)
