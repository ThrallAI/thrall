-- Schema v1, oversight: why a session was flagged for the owner (something its person's controls
-- didn't allow), or NULL. With "summary and flagged chats" the owner can read flagged ones.

ALTER TABLE sessions ADD COLUMN flagged TEXT;
