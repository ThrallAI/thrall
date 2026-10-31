-- Schema v1, phase 5 (docs/storage.md §3 Memory): past conversations (episodes), embeddings for
-- searching by meaning, and one full-text index over current facts and episodes.

CREATE TABLE episodes (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL,
    session_id  TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    summary     TEXT NOT NULL,
    content     TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX episodes_by_user ON episodes (user_id, created_at DESC);

-- One vector per remembered thing, stored as little-endian f32 bytes.
CREATE TABLE embeddings (
    owner_kind  TEXT NOT NULL,
    owner_id    TEXT NOT NULL,
    user_id     TEXT NOT NULL,
    model       TEXT NOT NULL,
    vector      BLOB NOT NULL,
    PRIMARY KEY (owner_kind, owner_id)
) WITHOUT ROWID;
CREATE INDEX embeddings_by_user ON embeddings (user_id, model);

CREATE VIRTUAL TABLE memory_fts USING fts5(
    owner_kind UNINDEXED,
    owner_id UNINDEXED,
    user_id UNINDEXED,
    text
);

-- Current facts are searchable; superseded or deleted ones drop out.
CREATE TRIGGER knowledge_fts_insert AFTER INSERT ON knowledge
WHEN NEW.superseded_by IS NULL
BEGIN
    INSERT INTO memory_fts (owner_kind, owner_id, user_id, text)
    VALUES ('knowledge', NEW.id, NEW.user_id, NEW.topic || ': ' || NEW.content);
END;

CREATE TRIGGER knowledge_fts_superseded AFTER UPDATE OF superseded_by ON knowledge
WHEN NEW.superseded_by IS NOT NULL
BEGIN
    DELETE FROM memory_fts WHERE owner_kind = 'knowledge' AND owner_id = NEW.id;
    DELETE FROM embeddings WHERE owner_kind = 'knowledge' AND owner_id = NEW.id;
END;

CREATE TRIGGER knowledge_fts_delete AFTER DELETE ON knowledge
BEGIN
    DELETE FROM memory_fts WHERE owner_kind = 'knowledge' AND owner_id = OLD.id;
    DELETE FROM embeddings WHERE owner_kind = 'knowledge' AND owner_id = OLD.id;
END;

CREATE TRIGGER episodes_fts_insert AFTER INSERT ON episodes
BEGIN
    INSERT INTO memory_fts (owner_kind, owner_id, user_id, text)
    VALUES ('episode', NEW.id, NEW.user_id, NEW.summary || ' ' || NEW.content);
END;

CREATE TRIGGER episodes_fts_delete AFTER DELETE ON episodes
BEGIN
    DELETE FROM memory_fts WHERE owner_kind = 'episode' AND owner_id = OLD.id;
    DELETE FROM embeddings WHERE owner_kind = 'episode' AND owner_id = OLD.id;
END;

-- Facts saved before this migration become searchable too.
INSERT INTO memory_fts (owner_kind, owner_id, user_id, text)
SELECT 'knowledge', id, user_id, topic || ': ' || content FROM knowledge WHERE superseded_by IS NULL;
