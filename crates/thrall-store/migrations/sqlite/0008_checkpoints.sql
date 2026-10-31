-- Schema v1, phase 8 step 3: checkpoints. Before a turn first changes a file, its old content is
-- kept here (or that it didn't exist), so the person can rewind to before any of their messages.
-- `after_hash` is the SHA-256 of what Thrall last left in the file in that turn: a rewind can tell
-- whether the person changed the file since.

CREATE TABLE checkpoints (
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    turn_id     TEXT NOT NULL,
    user_id     TEXT NOT NULL,
    path        TEXT NOT NULL,
    existed     INTEGER NOT NULL,
    content     BLOB,
    too_big     INTEGER NOT NULL DEFAULT 0,
    after_hash  TEXT,
    at          INTEGER NOT NULL,
    PRIMARY KEY (session_id, turn_id, path)
);
