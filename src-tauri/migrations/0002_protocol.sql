-- 0002: a host can speak Telnet instead of SSH (could-have M4). Existing hosts stay SSH.
ALTER TABLE hosts ADD COLUMN protocol TEXT NOT NULL DEFAULT 'ssh' CHECK (protocol IN ('ssh','telnet'));
