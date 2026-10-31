-- Schema v1, M1a: people. Each user has a role (`owner` set the engine up and runs it; `member`
-- uses it) and a plan (`free`; a hook for paid plans later). Both are text so new roles and plans
-- need no schema change. The local user, there before people existed, is the owner.

ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'member';
ALTER TABLE users ADD COLUMN plan TEXT NOT NULL DEFAULT 'free';
UPDATE users SET role = 'owner' WHERE id = 'u_local';
