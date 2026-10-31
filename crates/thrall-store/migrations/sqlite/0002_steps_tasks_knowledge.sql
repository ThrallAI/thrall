-- Schema v1, phase 2 subset (docs/storage.md §3): tool steps, the visible task list, and the
-- knowledge table the memory tools write to (embeddings and full-text search arrive in phase 5).

CREATE TABLE steps (
    id          TEXT PRIMARY KEY,
    turn_id     TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
    user_id     TEXT NOT NULL,
    tool        TEXT NOT NULL,
    title       TEXT NOT NULL,
    args        TEXT NOT NULL,
    status      TEXT NOT NULL,
    summary     TEXT NOT NULL DEFAULT '',
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER
);
CREATE INDEX steps_by_turn ON steps (turn_id, started_at);

CREATE TABLE tasks (
    id          TEXT PRIMARY KEY,
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    user_id     TEXT NOT NULL,
    text        TEXT NOT NULL,
    status      TEXT NOT NULL,
    position    INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX tasks_by_session ON tasks (session_id, position);

CREATE TABLE knowledge (
    id             TEXT PRIMARY KEY,
    user_id        TEXT NOT NULL REFERENCES users(id),
    topic          TEXT NOT NULL,
    content        TEXT NOT NULL,
    source         TEXT NOT NULL DEFAULT '{}',
    confidence     REAL NOT NULL DEFAULT 1.0,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    superseded_by  TEXT
);
CREATE INDEX knowledge_by_user ON knowledge (user_id, updated_at DESC);
