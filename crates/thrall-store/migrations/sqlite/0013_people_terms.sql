-- Schema v1, M2e/M2f: what each person's turns run on and may spend. `own_key` 1: their turns use
-- their own AI key (kept in the keychain under their name), not the owner's. `model`: their own
-- model (NULL: the engine's). `monthly_limit_micros`: the most their turns may cost in a calendar
-- month, in millionths of a dollar (NULL: no limit).

ALTER TABLE users ADD COLUMN own_key INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN model TEXT;
ALTER TABLE users ADD COLUMN monthly_limit_micros INTEGER;
