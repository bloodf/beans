-- Derived search tables, indexes, and the triggers that keep them in step with `messages`.
-- Runs on every open inside one transaction (`search.rs`); `search_drop.sql` runs first when
-- `SEARCH_INDEX_VERSION` changes, so no table here needs a migration. Every trigger is dropped
-- and created again each time, so a changed definition always takes effect.

-- Exact search: one row per message, its searchable fields folded as the matcher folds them.
CREATE TABLE IF NOT EXISTS message_search (
    message_id       TEXT PRIMARY KEY NOT NULL,
    chat_id          TEXT NOT NULL,
    created_at       REAL NOT NULL,
    position         INTEGER NOT NULL,
    stale            INTEGER NOT NULL DEFAULT 1 CHECK(stale IN (0,1)),
    text             TEXT NOT NULL DEFAULT '',
    files            TEXT NOT NULL DEFAULT '',
    -- Image attachment ids, newline-separated: what the image index and the describer read.
    images           TEXT NOT NULL DEFAULT '',
    -- Semantic search queues: text and file names, images, image descriptions. Each change to
    -- what a queue embeds bumps its version; the indexer saves only if the version it read holds.
    embed_pending    INTEGER NOT NULL DEFAULT 0 CHECK(embed_pending IN (0,1)),
    image_pending    INTEGER NOT NULL DEFAULT 0 CHECK(image_pending IN (0,1)),
    describe_pending INTEGER NOT NULL DEFAULT 0 CHECK(describe_pending IN (0,1)),
    embed_version    INTEGER NOT NULL DEFAULT 0,
    image_version    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS message_search_stale ON message_search(message_id) WHERE stale = 1;
-- Walking in result order lets a common term stop at the limit instead of sorting.
CREATE INDEX IF NOT EXISTS message_search_order ON message_search(created_at DESC, chat_id, position DESC, message_id);
-- Each queue is read newest first straight from its index: a pass costs its batch, not the backlog.
CREATE INDEX IF NOT EXISTS message_search_embed_queue ON message_search(created_at DESC, message_id) WHERE embed_pending = 1;
CREATE INDEX IF NOT EXISTS message_search_image_queue ON message_search(created_at DESC, message_id) WHERE image_pending = 1;
CREATE INDEX IF NOT EXISTS message_search_describe_queue ON message_search(created_at DESC, message_id) WHERE describe_pending = 1;
-- The few messages with pictures: whether any exist, and which show one that just arrived.
CREATE INDEX IF NOT EXISTS message_search_pictures ON message_search(message_id) WHERE images != '';

-- Semantic search: int8 chunk vectors, one space per role. AUTOINCREMENT ids never repeat, so
-- an in-memory copy reads only rows past the last id it holds, and the deletion log says which
-- rows went.
CREATE TABLE IF NOT EXISTS message_vectors (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT NOT NULL,
    role       TEXT NOT NULL CHECK(role IN ('text','image')),
    chunk      INTEGER NOT NULL,
    space      TEXT NOT NULL,
    scale      REAL NOT NULL,
    vector     BLOB NOT NULL,
    -- Roles number chunks from disjoint bases (`search_index::*_CHUNK_BASE`).
    UNIQUE(message_id, chunk)
);
-- Progress counts a role's messages without reading its vectors.
CREATE INDEX IF NOT EXISTS message_vectors_role ON message_vectors(role, message_id);
CREATE TABLE IF NOT EXISTS message_vector_deletions (
    seq       INTEGER PRIMARY KEY AUTOINCREMENT,
    vector_id INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS message_vector_spaces (
    role  TEXT PRIMARY KEY NOT NULL CHECK(role IN ('text','image')),
    space TEXT NOT NULL
);

-- Writes to `messages`, from this binary or any other, reach the projection.
DROP TRIGGER IF EXISTS message_search_insert;
CREATE TRIGGER message_search_insert AFTER INSERT ON messages BEGIN
    INSERT OR REPLACE INTO message_search(message_id, chat_id, created_at, position)
    VALUES (NEW.id, NEW.chat_id, NEW.created_at, NEW.position);
END;
DROP TRIGGER IF EXISTS message_search_update;
CREATE TRIGGER message_search_update AFTER UPDATE ON messages BEGIN
    DELETE FROM message_search WHERE message_id = OLD.id AND OLD.id IS NOT NEW.id;
    INSERT INTO message_search(message_id, chat_id, created_at, position)
    VALUES (NEW.id, NEW.chat_id, NEW.created_at, NEW.position)
    ON CONFLICT(message_id) DO UPDATE SET
        chat_id = excluded.chat_id, created_at = excluded.created_at, position = excluded.position,
        stale = CASE WHEN OLD.id IS NEW.id AND OLD.message_json IS NEW.message_json THEN stale ELSE 1 END;
END;
DROP TRIGGER IF EXISTS message_search_delete;
CREATE TRIGGER message_search_delete AFTER DELETE ON messages BEGIN
    DELETE FROM message_search WHERE message_id = OLD.id;
    DELETE FROM image_descriptions WHERE message_id = OLD.id;
END;

-- Vectors never outlive what they were made from: new text or file names drop the message's
-- text vectors and queue it; a new image list drops its image vectors and queues both image
-- queues; a deleted message takes everything along.
DROP TRIGGER IF EXISTS message_search_queue_new;
CREATE TRIGGER message_search_queue_new AFTER INSERT ON message_search
WHEN NEW.text != '' OR NEW.files != '' OR NEW.images != '' BEGIN
    UPDATE message_search SET
        embed_pending = (NEW.text != '' OR NEW.files != ''),
        image_pending = (NEW.images != ''),
        describe_pending = (NEW.images != '')
    WHERE message_id = NEW.message_id;
END;
DROP TRIGGER IF EXISTS message_search_queue_text;
CREATE TRIGGER message_search_queue_text AFTER UPDATE OF text, files ON message_search
WHEN OLD.text IS NOT NEW.text OR OLD.files IS NOT NEW.files BEGIN
    DELETE FROM message_vectors WHERE message_id = NEW.message_id AND role = 'text';
    UPDATE message_search SET embed_pending = (NEW.text != '' OR NEW.files != ''), embed_version = embed_version + 1
    WHERE message_id = NEW.message_id;
END;
DROP TRIGGER IF EXISTS message_search_queue_images;
CREATE TRIGGER message_search_queue_images AFTER UPDATE OF images ON message_search
WHEN OLD.images IS NOT NEW.images BEGIN
    DELETE FROM message_vectors WHERE message_id = NEW.message_id AND role = 'image';
    UPDATE message_search SET image_pending = (NEW.images != ''), describe_pending = (NEW.images != ''),
        image_version = image_version + 1
    WHERE message_id = NEW.message_id;
END;
DROP TRIGGER IF EXISTS message_search_drop_vectors;
CREATE TRIGGER message_search_drop_vectors AFTER DELETE ON message_search BEGIN
    DELETE FROM message_vectors WHERE message_id = OLD.message_id;
END;
DROP TRIGGER IF EXISTS message_vectors_log_deletion;
CREATE TRIGGER message_vectors_log_deletion AFTER DELETE ON message_vectors BEGIN
    INSERT INTO message_vector_deletions(vector_id) VALUES (OLD.id);
END;

-- A description is more text for the message: embed it again.
DROP TRIGGER IF EXISTS image_descriptions_queue;
CREATE TRIGGER image_descriptions_queue AFTER INSERT ON image_descriptions BEGIN
    UPDATE message_search SET embed_pending = 1, embed_version = embed_version + 1 WHERE message_id = NEW.message_id;
END;
