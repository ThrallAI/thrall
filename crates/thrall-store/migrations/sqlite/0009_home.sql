-- Schema v1, phase 8 step 6: Home, Thrall's own built-in conversation. Each page of it is a
-- session with `home_day` set (the local date it started); the newest is the current page. Home
-- pages never appear in the session list.

ALTER TABLE sessions ADD COLUMN home_day TEXT;
CREATE INDEX sessions_home ON sessions (user_id, home_day) WHERE home_day IS NOT NULL;
