UPDATE task_invocations SET state='dismissed' WHERE state IN ('pending','authorized')
