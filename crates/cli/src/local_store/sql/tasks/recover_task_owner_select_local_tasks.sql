SELECT EXISTS(SELECT 1 FROM local_tasks UNION ALL SELECT 1 FROM task_fences UNION ALL SELECT 1 FROM task_invocations UNION ALL SELECT 1 FROM task_effect_receipts)
