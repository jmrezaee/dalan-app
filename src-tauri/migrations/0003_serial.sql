-- 0003: serial-port hosts (could-have M4). The device path is the host's `address`; line settings live here.
-- SQLite can't change a CHECK, so `protocol` is rebuilt (existing values are copied) to also allow 'serial'.
ALTER TABLE hosts ADD COLUMN protocol_new TEXT NOT NULL DEFAULT 'ssh' CHECK (protocol_new IN ('ssh','telnet','serial'));
UPDATE hosts SET protocol_new = protocol;
ALTER TABLE hosts DROP COLUMN protocol;
ALTER TABLE hosts RENAME COLUMN protocol_new TO protocol;

CREATE TABLE serial_settings (
  host_id    TEXT PRIMARY KEY REFERENCES hosts(id) ON DELETE CASCADE,
  baud       INTEGER NOT NULL DEFAULT 9600 CHECK (baud BETWEEN 50 AND 4000000),
  data_bits  INTEGER NOT NULL DEFAULT 8 CHECK (data_bits BETWEEN 5 AND 8),
  stop_bits  INTEGER NOT NULL DEFAULT 1 CHECK (stop_bits IN (1,2)),
  parity     TEXT NOT NULL DEFAULT 'none' CHECK (parity IN ('none','odd','even')),
  flow       TEXT NOT NULL DEFAULT 'none' CHECK (flow IN ('none','hardware','software'))
);
