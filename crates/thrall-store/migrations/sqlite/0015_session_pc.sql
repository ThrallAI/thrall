-- Schema v1, companion PCs: the PC (a joined device's id) whose session it is. Its tools run on
-- that PC, never on this computer. Empty for sessions that work here.

ALTER TABLE sessions ADD COLUMN pc TEXT;
