-- Schema v1, phase 3 subset (docs/storage.md §3): "always" answers, and the append-only audit log
-- every gate decision is written to.

CREATE TABLE grants (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id),
    session_id  TEXT REFERENCES sessions(id) ON DELETE CASCADE,
    tool        TEXT NOT NULL,
    scope       TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER
);
CREATE INDEX grants_by_session ON grants (user_id, session_id, scope);

CREATE TABLE audit_log (
    id        TEXT PRIMARY KEY,
    user_id   TEXT NOT NULL,
    at        INTEGER NOT NULL,
    gate      TEXT NOT NULL,
    action    TEXT NOT NULL,
    decision  TEXT NOT NULL,
    detail    TEXT NOT NULL
);
CREATE INDEX audit_by_user_time ON audit_log (user_id, at);

CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'the audit log is append-only');
END;

CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'the audit log is append-only');
END;
