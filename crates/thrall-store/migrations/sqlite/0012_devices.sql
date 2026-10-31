-- Schema v1, M2c: signed-in browsers and PCs. A one-time sign-in link turns into a device token;
-- only the token's SHA-256 is kept (a copied database can't sign anyone in). Signing a device out
-- sets `removed_at`; the row stays as a record.

CREATE TABLE devices (
    id            TEXT PRIMARY KEY,
    user_id       TEXT NOT NULL REFERENCES users(id),
    name          TEXT NOT NULL,
    token_hash    TEXT NOT NULL UNIQUE,
    created_at    INTEGER NOT NULL,
    last_seen_at  INTEGER,
    removed_at    INTEGER
);

CREATE INDEX devices_user ON devices(user_id);
