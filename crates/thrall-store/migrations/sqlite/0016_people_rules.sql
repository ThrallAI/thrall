-- Schema v1, controls: what the owner allows each person, as JSON (thrall_protocol::PersonRules).
-- Everything is allowed until the owner turns something off: '{}'.

ALTER TABLE users ADD COLUMN rules TEXT NOT NULL DEFAULT '{}';
