SELECT task_id,chat_id,bot_id,routine_id,state FROM local_tasks WHERE account_epoch=?1 AND (?2 IS NULL OR task_id>?2) ORDER BY task_id LIMIT ?3
