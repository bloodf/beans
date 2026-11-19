DELETE FROM task_invocations WHERE EXISTS(SELECT 1 FROM local_tasks t WHERE t.account_epoch=task_invocations.account_epoch AND t.task_id=task_invocations.task_id AND t.chat_id=?1)
