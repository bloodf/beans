UPDATE routine_schedule_authority SET next_at=?4 WHERE routine_id=?1 AND binding=?2 AND revoked=0 AND next_at<?3
