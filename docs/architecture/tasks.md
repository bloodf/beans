# Local task authority

The CLI execution owner holds an exclusive OS lock on `execution.lock` for the App lifetime. `App::load_owner` acquires it before opening SQLite or recovering submitted work. Busy or unsupported locking rejects startup. `beans serve` uses this mode. `App::load` remains a non-executing Device view and does not recover the memory delivery queue. Owner startup retains submitted-to-unknown memory recovery.

SQLite uses WAL, FULL synchronization, fullfsync and checkpoint_fullfsync. Safety APIs verify the connection's durability settings under its mutex and use immediate transactions. Disk guarantees depend on the filesystem and hardware honoring sync; these settings do not prove power-loss durability.

## Admission and recovery

Each local Job uses its existing `Job.id` as task identity, scoped to the persisted account epoch. Room coordinator ids are not tasks. Queueing inserts an account-lifetime replay fence and task together before spawning. Duplicate jobs are refused regardless of task state or deleted detail. Local background jobs, room member turns, and sealed sync jobs share admission. Sync propagates admission storage failures before acknowledging its page cursor.

Task leases capture account epoch, owner epoch, execution id and the App's existing plugin account incarnation. Start changes only the exact queued task to running when cancellation has not committed. Stop persists cancellation before cancelling live tokens. Persistence failure closes in-process account admission. Forget closes durable task authority before clearing account data and changes the existing incarnation; callbacks from the forgotten account cannot recreate receipts.

Owner recovery changes started effect receipts to unknown and closes pending or authorized process-local invocations as dismissed. Queued or running tasks with any receipt become needs_review; receipt-free tasks become interrupted. Neither resumes automatically. Interrupted or needs_review routine work blocks later scheduled admission rather than disguising recovery as a new run.

Existing populated stores missing task safety authority reject owner startup. No production migration, restored-home reconstruction or live rollout is performed by this source mechanism.

## Invocation API

`App::task_execution` returns the active captured lease. `freeze_task_invocation` inserts an immutable host-issued invocation/attempt/receipt identity, parent ordinal, revision and 32-byte digest. Authorization is exact-binding pending-state CAS to authorized, denied, dismissed or expired, with enumerated kind and optional structural card/answering Device/time. Duplicate answers cannot reopen closure.

`admit_task_invocation` holds the roster policy boundary and existing account incarnation boundary, invokes the host's synchronous final recheck, and atomically consumes authorized state plus inserts a started receipt. Any commit error forbids dispatch, including ambiguous acknowledgment. The host must validate effective capabilities, concrete installation, target/workspace and immutable digest after preparation awaits. The effect-execution owner (#111) supplies that recheck and connects consequential tool paths; the current agent/tool pipeline is not yet wired to these APIs.

`finish_task_receipt` updates only an existing exact started receipt. Finished means observed end; DefinitelyNotSent requires typed definitive no-dispatch evidence; uncertain outcomes are unknown. Unknown is immutable. No callback upserts, refines recovered unknown, or creates deleted history. Finishing a task retains needs_review when any receipt is started or unknown.

## Routine checks and retention

Scheduled routine checks use a staging CodemodeStore. Check writes, check timestamp, queued task and check report commit together. A rejected transaction leaves no consumed store values or queued job. The scheduler starts only the already-admitted job after commit. Explicit check execution commits staged values and timestamp without manufacturing a scheduled task.

Explicit resolution records a timestamp only when no receipt remains started. Detail pruning removes resolved history after 30 days; unresolved detail stays. Replay fences remain for the account lifetime. Chat deletion removes task/invocation/receipt detail while retaining fences; account forget removes both with account data. Missing detail is not permission for a workflow to rerun a step. Workflow approval lifecycle belongs to its separate owner.

Receipt records contain structural identities, enumerated states and keyed digest bindings, not arguments, raw exceptions, tool results or credentials. Routine check reports are task detail, not receipt projections. No receipt export or relay projection is added. SQLite WAL/SHM, backups and snapshots may retain prior bytes; DELETE is not secure erasure.
