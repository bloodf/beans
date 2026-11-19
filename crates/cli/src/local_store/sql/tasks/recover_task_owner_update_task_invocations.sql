UPDATE task_invocations SET state='dismissed' WHERE account_epoch=?1 AND state IN ('pending','authorized')
