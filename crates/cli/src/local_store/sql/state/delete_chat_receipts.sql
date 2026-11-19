DELETE FROM task_effect_receipts WHERE EXISTS(SELECT 1 FROM local_tasks t WHERE t.account_epoch=task_effect_receipts.account_epoch AND t.task_id=task_effect_receipts.task_id AND t.chat_id=?1)
