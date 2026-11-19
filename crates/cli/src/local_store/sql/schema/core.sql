PRAGMA journal_mode = WAL;
PRAGMA fullfsync = ON;
PRAGMA checkpoint_fullfsync = ON;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA journal_size_limit = 16777216;
CREATE TABLE IF NOT EXISTS metadata (
    id                       INTEGER PRIMARY KEY CHECK (id = 1),
    auto_review_json         TEXT NOT NULL,
    last_seq                 INTEGER NOT NULL,
    roster_slot_seq          INTEGER NOT NULL DEFAULT 0,
    machine_blob_hash        TEXT,
    credentials_uploaded     INTEGER NOT NULL,
    paused                   INTEGER NOT NULL DEFAULT 0,
    policy_json              TEXT NOT NULL DEFAULT '{}'
);
CREATE TABLE IF NOT EXISTS devices (
    id       TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    json     TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS bots (
    id       TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    json     TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS chats (
    id       TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    json     TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS routines (
    id       TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    json     TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS workflows (
    id       TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    json     TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS group_deletes (
    id       TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS pending_chat_creates (
    id TEXT PRIMARY KEY NOT NULL,
    original_json TEXT
);
CREATE TABLE IF NOT EXISTS roster_baseline (
    name TEXT PRIMARY KEY NOT NULL,
    seq INTEGER NOT NULL,
    json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS device_names (
    id TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS blob_deletes (
    id       TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS device_seen (
    id      TEXT PRIMARY KEY NOT NULL,
    seen_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS applied_blobs (
    id       TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS chat_history (
    chat_id      TEXT PRIMARY KEY NOT NULL,
    before_place INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
    id            TEXT PRIMARY KEY NOT NULL,
    chat_id       TEXT NOT NULL,
    position      INTEGER NOT NULL,
    sort_at       REAL NOT NULL,
    created_at    REAL NOT NULL,
    author_kind   TEXT NOT NULL,
    author_bot_id TEXT,
    body_kind     TEXT NOT NULL,
    is_complete   INTEGER NOT NULL,
    text_nonempty INTEGER NOT NULL,
    message_json  TEXT NOT NULL,
    UNIQUE(chat_id, position)
);
CREATE INDEX IF NOT EXISTS messages_chat_position
    ON messages(chat_id, position);
CREATE INDEX IF NOT EXISTS messages_chat_order
    ON messages(chat_id, sort_at, position);
CREATE INDEX IF NOT EXISTS messages_author
    ON messages(author_kind, author_bot_id, created_at);
CREATE TABLE IF NOT EXISTS outbox (
    position        INTEGER PRIMARY KEY AUTOINCREMENT,
    id              TEXT UNIQUE NOT NULL,
    kind            TEXT NOT NULL,
    recipient       TEXT,
    ciphertext      BLOB NOT NULL,
    slot_name       TEXT,
    slot_keep_first INTEGER NOT NULL DEFAULT 0,
    group_name      TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS outbox_slot
    ON outbox(slot_name) WHERE slot_name IS NOT NULL;
CREATE TABLE IF NOT EXISTS codemode_store (
    chat_id TEXT NOT NULL,
    bot_id  TEXT NOT NULL,
    key     TEXT NOT NULL,
    json    TEXT NOT NULL,
    PRIMARY KEY (chat_id, bot_id, key)
);
CREATE TABLE IF NOT EXISTS device_turns (
    id   TEXT PRIMARY KEY NOT NULL,
    json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sent_jobs (
    id         TEXT PRIMARY KEY NOT NULL,
    chat_id    TEXT NOT NULL,
    bot_id     TEXT NOT NULL,
    routine_id TEXT,
    runner_id  TEXT NOT NULL,
    sent_at    REAL NOT NULL
);
CREATE TABLE IF NOT EXISTS memory_config (
    id INTEGER PRIMARY KEY CHECK(id=1), ciphertext BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS memory_deliveries (
    id TEXT PRIMARY KEY NOT NULL, bot_id TEXT NOT NULL,
    state TEXT NOT NULL, json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS memory_deliveries_bot ON memory_deliveries(bot_id,state);
CREATE TABLE IF NOT EXISTS memory_fences (
    bot_id TEXT PRIMARY KEY NOT NULL, json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS memory_runner_bindings (
    key TEXT PRIMARY KEY NOT NULL, json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS memory_turn_admissions (
    job_id TEXT PRIMARY KEY NOT NULL, json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS task_authority (
    id INTEGER PRIMARY KEY CHECK(id=1), account_epoch TEXT NOT NULL,
    owner_epoch TEXT NOT NULL, closed INTEGER NOT NULL CHECK(closed IN (0,1)), admission_floor REAL);
CREATE TABLE IF NOT EXISTS task_safety_version(id INTEGER PRIMARY KEY CHECK(id=1), version INTEGER NOT NULL CHECK(version=1));
CREATE TABLE IF NOT EXISTS task_rejected_envelopes (
    account_identity TEXT NOT NULL, task_id TEXT NOT NULL, PRIMARY KEY(account_identity,task_id));
CREATE TABLE IF NOT EXISTS task_fences (
    account_epoch TEXT NOT NULL, task_id TEXT NOT NULL, PRIMARY KEY(account_epoch,task_id));
CREATE TABLE IF NOT EXISTS local_tasks (
    account_epoch TEXT NOT NULL, task_id TEXT NOT NULL, owner_epoch TEXT NOT NULL,
    execution_id TEXT NOT NULL, chat_id TEXT NOT NULL, bot_id TEXT NOT NULL, routine_id TEXT,
    state TEXT NOT NULL CHECK(state IN ('queued','running','finished','interrupted','needs_review')),
    cancel_requested INTEGER NOT NULL DEFAULT 0, resolved_at INTEGER, check_report TEXT, PRIMARY KEY(account_epoch,task_id));
CREATE TABLE IF NOT EXISTS task_invocations (
    account_epoch TEXT NOT NULL, task_id TEXT NOT NULL, owner_epoch TEXT NOT NULL,
    execution_id TEXT NOT NULL, invocation_id TEXT NOT NULL, parent_id TEXT,
    ordinal INTEGER NOT NULL, attempt_id TEXT NOT NULL, receipt_id TEXT NOT NULL,
    revision INTEGER NOT NULL, digest BLOB NOT NULL CHECK(length(digest)=32),
    state TEXT NOT NULL CHECK(state IN ('pending','authorized','denied','dismissed','expired','admitted')),
    kind TEXT, card_id TEXT, answering_device TEXT, answered_at INTEGER,
    PRIMARY KEY(account_epoch,task_id,attempt_id), UNIQUE(account_epoch,receipt_id),
    UNIQUE(account_epoch,task_id,execution_id,invocation_id));
CREATE UNIQUE INDEX IF NOT EXISTS task_invocation_ordinal ON task_invocations(account_epoch,task_id,execution_id,COALESCE(parent_id,''),ordinal);
CREATE TABLE IF NOT EXISTS task_effect_receipts (
    account_epoch TEXT NOT NULL, task_id TEXT NOT NULL, owner_epoch TEXT NOT NULL,
    execution_id TEXT NOT NULL, attempt_id TEXT NOT NULL, receipt_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('started','finished','failed','unknown')), PRIMARY KEY(account_epoch,receipt_id));
CREATE TABLE IF NOT EXISTS routine_check_commits (
    account_epoch TEXT NOT NULL, routine_id TEXT NOT NULL, checked_at INTEGER NOT NULL,
    PRIMARY KEY(account_epoch,routine_id));
CREATE TABLE IF NOT EXISTS routine_schedule_authority (
    account_epoch TEXT NOT NULL, routine_id TEXT NOT NULL, revision TEXT NOT NULL,
    binding TEXT NOT NULL, next_at INTEGER NOT NULL, revoked INTEGER NOT NULL CHECK(revoked IN (0,1)), PRIMARY KEY(account_epoch,routine_id));
CREATE TABLE IF NOT EXISTS routine_schedule_occurrences (
    account_epoch TEXT NOT NULL, routine_id TEXT NOT NULL, revision TEXT NOT NULL,
    occurrence INTEGER NOT NULL, PRIMARY KEY(account_epoch,routine_id,revision,occurrence));
-- Exact and semantic search keep derived tables (`search.sql`), rebuilt when this changes.
CREATE TABLE IF NOT EXISTS message_search_version (
    id INTEGER PRIMARY KEY CHECK(id = 1), version INTEGER NOT NULL
);
-- What a vision model said an image shows, embedded with the message's text.
CREATE TABLE IF NOT EXISTS image_descriptions (
    message_id    TEXT NOT NULL,
    attachment_id TEXT NOT NULL,
    model         TEXT NOT NULL,
    text          TEXT NOT NULL,
    PRIMARY KEY(message_id, attachment_id)
);
PRAGMA user_version = 4;
