-- Schema v1, phase 4 subset (docs/storage.md §3): background helpers and scheduled jobs.

CREATE TABLE agents (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL,
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    status      TEXT NOT NULL,
    state       TEXT NOT NULL DEFAULT '{}',
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER
);
CREATE INDEX agents_by_session ON agents (session_id, started_at);

CREATE TABLE jobs (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users(id),
    session_id   TEXT REFERENCES sessions(id) ON DELETE CASCADE,
    title        TEXT NOT NULL,
    schedule     TEXT NOT NULL,
    prompt       TEXT NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 1,
    last_run_at  INTEGER,
    next_run_at  INTEGER NOT NULL
);
CREATE INDEX jobs_due ON jobs (user_id, enabled, next_run_at);

CREATE TABLE job_runs (
    id          TEXT PRIMARY KEY,
    job_id      TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    user_id     TEXT NOT NULL,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER,
    status      TEXT NOT NULL,
    summary     TEXT NOT NULL DEFAULT ''
);
CREATE INDEX job_runs_by_job ON job_runs (job_id, started_at);
