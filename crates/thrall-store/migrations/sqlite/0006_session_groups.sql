-- Schema v1, phase 6b: the person's own session groups (named, ordered) and pinned sessions.

CREATE TABLE session_groups (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id),
    name        TEXT NOT NULL,
    position    INTEGER NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX session_groups_by_user ON session_groups (user_id, position);

-- Deleting a group leaves its sessions where they were, just ungrouped.
ALTER TABLE sessions ADD COLUMN group_id TEXT REFERENCES session_groups(id) ON DELETE SET NULL;
ALTER TABLE sessions ADD COLUMN pinned_at INTEGER;
