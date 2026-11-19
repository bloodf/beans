INSERT INTO routine_check_commits VALUES(?1,?2,?3) ON CONFLICT(account_epoch,routine_id) DO UPDATE SET checked_at=excluded.checked_at
