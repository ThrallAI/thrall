-- Schema v1, phase 1 subset (docs/storage.md §3): users, sessions, the event log, turns,
-- model messages and usage. Later phases add their tables in new migrations.

CREATE TABLE users (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE TABLE sessions (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users(id),
    title        TEXT NOT NULL DEFAULT '',
    workspace    TEXT,
    mode         TEXT NOT NULL DEFAULT 'ask',
    state        TEXT NOT NULL DEFAULT 'idle',
    last_seq     INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    archived_at  INTEGER
);
CREATE INDEX sessions_by_user_updated ON sessions (user_id, updated_at DESC);

-- What people saw, exactly as sent. Append-only; replayed by session.open {after_seq}.
CREATE TABLE events (
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    seq         INTEGER NOT NULL,
    user_id     TEXT NOT NULL,
    turn_id     TEXT,
    type        TEXT NOT NULL,
    payload     TEXT NOT NULL,
    at          INTEGER NOT NULL,
    PRIMARY KEY (session_id, seq)
) WITHOUT ROWID;

CREATE TRIGGER events_no_update BEFORE UPDATE ON events
BEGIN
    SELECT RAISE(ABORT, 'events are append-only');
END;

-- Events may only disappear together with their session.
CREATE TRIGGER events_no_delete BEFORE DELETE ON events
WHEN EXISTS (SELECT 1 FROM sessions WHERE id = OLD.session_id)
BEGIN
    SELECT RAISE(ABORT, 'events are append-only');
END;

CREATE TABLE turns (
    id             TEXT PRIMARY KEY,
    session_id     TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    user_id        TEXT NOT NULL,
    status         TEXT NOT NULL,
    started_at     INTEGER NOT NULL,
    ended_at       INTEGER,
    input_tokens   INTEGER NOT NULL DEFAULT 0,
    output_tokens  INTEGER NOT NULL DEFAULT 0,
    cost_micros    INTEGER
);
CREATE INDEX turns_by_session ON turns (session_id, started_at);

-- What the model sees. Separate from events: context gets compacted, history must not.
CREATE TABLE messages (
    id          TEXT PRIMARY KEY,
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    user_id     TEXT NOT NULL,
    turn_id     TEXT,
    role        TEXT NOT NULL,
    content     TEXT NOT NULL,
    tokens      INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    compacted   INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX messages_by_session ON messages (session_id, created_at, id);

-- One row per model call. No foreign key on session_id: cost history outlives deleted sessions.
CREATE TABLE usage (
    id             TEXT PRIMARY KEY,
    user_id        TEXT NOT NULL,
    session_id     TEXT,
    turn_id        TEXT,
    provider       TEXT NOT NULL,
    model          TEXT NOT NULL,
    input_tokens   INTEGER NOT NULL,
    output_tokens  INTEGER NOT NULL,
    cost_micros    INTEGER,
    at             INTEGER NOT NULL
);
CREATE INDEX usage_by_user_time ON usage (user_id, at);
