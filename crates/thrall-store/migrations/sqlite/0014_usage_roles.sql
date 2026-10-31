-- Schema v1, split mode: who made each model call and how much of it was cached. `role`: `chat`
-- (an ordinary call), or in split mode `thinker`, `doer` or `ask` (the Doer asking the Thinker).
-- `cached_tokens`: how many of the tokens sent the provider had already seen (charged less).

ALTER TABLE usage ADD COLUMN role TEXT NOT NULL DEFAULT 'chat';
ALTER TABLE usage ADD COLUMN cached_tokens INTEGER NOT NULL DEFAULT 0;
