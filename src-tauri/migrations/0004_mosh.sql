-- 0004: Mosh hosts (could-have M4). A Mosh host signs in over SSH like any other, then hands over to mosh.
-- `protocol` is rebuilt (SQLite can't change a CHECK); existing values are copied.
ALTER TABLE hosts ADD COLUMN protocol_new TEXT NOT NULL DEFAULT 'ssh' CHECK (protocol_new IN ('ssh','telnet','serial','mosh'));
UPDATE hosts SET protocol_new = protocol;
ALTER TABLE hosts DROP COLUMN protocol;
ALTER TABLE hosts RENAME COLUMN protocol_new TO protocol;
