-- Schema v1, phase 6b: editing or retrying the last message takes its turn back. The turn's
-- context messages stay stored (history is never rewritten) but the model no longer sees them.

ALTER TABLE messages ADD COLUMN retracted_at INTEGER;
