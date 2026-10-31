-- Schema v1, M2a: removing a person. Their row, sessions and files stay (nothing a person made is
-- deleted by removing them); `removed_at` (Unix ms) only stops them connecting and drops them from
-- the people list.

ALTER TABLE users ADD COLUMN removed_at INTEGER;
