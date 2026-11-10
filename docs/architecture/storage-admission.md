# Local storage and startup admission

`crates/cli`, binary `beans`. Local websocket for the app. Files live under the private data directory: `~/.beans-v2/` by default and for standalone Beans, `~/.beans-dev-v2/` when launched by the Windows/Linux development app, or the explicit `--home` / `BEANS_HOME`. The macOS Beans bundles set `~/.beans-v2/` (port `4874`) and `~/.beans-dev-v2/` (port `4875`) respectively. It contains `identity.json` (master secret, identity devices only), `machine.json` (machine secret, identity and content public keys, account DEK, `name`, `os`), `credentials.json` (the account’s providers, as last merged), `settings.json` (relay URL), `beans.sqlite3` (WAL-mode plaintext local state), and `files/<attachment id>` (attachment bytes, sent from here or fetched from the relay). SQLite holds devices, bots, chat metadata, routines, Auto-review, usage, compactions, sync bookkeeping, messages, the durable relay outbox, the jobs this Device has asked other Runners to do, and pending plugin actions.

The home also holds private `mcp.json`, source-bound `catalog.json` and `marketplace.json` caches, and `plugins/`. JSON/database files are mode 0600 inside the mode-0700 home; SQLite keeps its WAL/SHM companions there. The retained updater's `update.json` is not read or mutated while CLI self-update is disabled. Shared private writes replace a file by renaming a same-directory temporary that is never reused or followed. On Unix that temporary carries mode 0600 from its creation, so no secret byte is written into a file another account can read, and a failed write or rename attempts to remove only the temporary that write created.

Storage validates the `beans-v2` marker before creation, permissions, deletion, recovery or SQLite setup. Explicit old/unmarked populated homes reject without modifying bytes, and malformed account records never become an empty identity. Direct database opens and mobile push decryption enforce the same boundary; see [fresh Beans format](identity.md#fresh-beans-format). Existing `.beans` / `.beans-dev` accounts are not migrated or reused.

The compiled `storage-upgrade <scratch-home>` command deliberately invokes the staged scratch preload before `App::load`, without identity, credentials or relay initialization. It requires an existing empty Beans v2 marked scratch home, publishes the staged marker and opens only its staged local store with FULL synchronous commits. The public staged store entry revalidates the bounded marker, syncs file and retained parent handles, and rechecks identity/content under the shared admission lock on every open. Symlinks, replacement, lock contention and durability uncertainty reject before database mutation. Unsupported non-Unix staged admission fails closed; ordinary non-Unix admission retains its existing behavior. Normal commands and global protocol floors remain unchanged; this explicit scratch mechanism is not live-account migration.

Home creation and mutable startup share one admission lifetime on Unix. `ensure_home_admission` returns its precreation guard to `App::load`, which passes it to local store admission without dropping or reacquiring it and releases it on return or error. The read-only home validator keeps its directory-check, preflight and legacy-marker ordering; exclusion lives outside that validator. Lock failures retain the OS error and report only the admission stage, shared/exclusive mode, error class and errno, without adding home paths or account identifiers. All lock failures still reject immediately, before format-marker writes or database mutation. The created-home lock is acquired after directory creation, so a failure at that stage may leave newly created directories in place.

`App::load` snapshots the normalized startup state, releases the state mutex, and propagates failure from the final local-store save before returning the App. That save remains inside the same owning admission lifetime as home creation and database setup; failed persistence does not become successful startup. Other best-effort state-save callers keep their existing behavior.

Configured absolute macOS `/var/...` homes support only the verified system `/var -> private/var` alias through descriptor-bound trusted-prefix admission. Relative paths retain current-directory semantics and receive no platform alias exception. The platform root namespace must remain privileged and stable; supplied home suffixes still open without following links. Scratch verification uses an explicit `/var` spelling of its owned physical temporary suffix as well as ordinary temporary-home consumers, rather than changing the historical CLI path to bypass admission.

## Retention dry-run

`LocalStore::retention_dry_run` reads one exact chat and returns a typed local
transcript plan. `RetentionPolicy` defaults to `Disabled`. An explicit `Before`
cutoff uses Unix seconds and excludes the boundary. Only complete, unqueued rows
older than that cutoff qualify; a later promotion also prevents selection.
Active or unknown chat admission returns no candidates. `OwnedAndIdle` describes
only the caller's snapshot and grants no execution authority. No runtime, API or
sweeper calls this planner, and no retention settings are persisted.

Every plan carries `InventoryIncomplete`. Candidate IDs describe local display
history only. There is no deletion executor or deletion receipt. A missing chat
returns an empty local list and does not prove absence on another Device or the
relay. The planner does not claim data was deleted everywhere.

The retention inventory is deliberately unresolved outside local message rows:

| Store | Current retention boundary |
| --- | --- |
| SQLite messages, compactions and chat usage | Only message IDs are planned; compactions and usage remain untouched. |
| `files/`, inline image JSON and workspace attachment copies | Shared references and ownership need reconciliation; no file deletion is planned. |
| Bot `MEMORY.md`, diary and embeddings | Derived text can combine chats; no age-based removal is planned. |
| Memory service deliveries, remote documents and banks | Existing revision/epoch deletion fences and uncertain deliveries remain authoritative; no remote request is made. |
| Relay chat/file blobs, group deletes and local outbox | Existing group-delete propagation is separate; this planner queues nothing and cannot attest relay or paired-Device removal. |
| Task/effect receipts, replay fences and tombstones | Preserved. Display-history deletion never authorizes stale task replay or reissue of unknown effects. |
| Managed policy receipts and policy history | Preserved by their existing storage boundaries. |
| SQLite WAL/SHM, backups, snapshots and exported copies | Residual copies; SQL deletion would not constitute secure erasure. |

The issue 109 safety contract remains a prerequisite for any executor. It requires
account-lifetime minimal `(account_epoch, task_id, replay-denial marker)`
tombstones without TTL, preserving unresolved started/unknown/needs-review
records. Explicitly resolved details have a separate 30-day retention rule.
These structural identifiers reveal task linkage and survive display-history
removal until account forget closes the epoch. They carry no tool arguments or
result text. The planner neither implements nor relaxes that contract.

Any future irreversible operation must confirm the exact account epoch, chat,
cutoff, selected IDs, dependent stores and current plan revision, then revalidate
ownership and inactive turns under the admission fence. A preview or generic
confirmation cannot authorize it. Unknown store ownership, unresolved effects or
incomplete inventory must block execution. Account teardown remains a separate
operation. This dry-run supplies no such confirmation or authority.
