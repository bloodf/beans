SELECT checked_at FROM routine_check_commits WHERE routine_id=?1 AND account_epoch=(SELECT account_epoch FROM task_authority WHERE id=1 AND closed=0)
