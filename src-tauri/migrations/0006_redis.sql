-- 0006: Redis (replica/redis/spec.md section 2). Saved databases for the Databases section (S26, S27) and the
-- console history of each one (used from R4). A database is reached directly or through a saved host.
CREATE TABLE redis_databases (
  id               TEXT PRIMARY KEY,                              -- UUID v7
  vault_id         TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  name             TEXT NOT NULL,
  environment      TEXT NOT NULL DEFAULT 'none' CHECK (environment IN ('none','staging','production')),
  route            TEXT NOT NULL CHECK (route IN ('direct','host')),
  host_id          TEXT REFERENCES hosts(id) ON DELETE RESTRICT,  -- required when route = 'host'
  address          TEXT NOT NULL,     -- host name or IP as seen from the route's last hop; for Sentinel, a comma list
  port             INTEGER NOT NULL DEFAULT 6379,
  kind             TEXT NOT NULL DEFAULT 'single' CHECK (kind IN ('single','cluster','sentinel')),
  sentinel_master  TEXT,              -- required when kind = 'sentinel'
  username         TEXT,
  password_secret_id          TEXT REFERENCES secrets(id),
  sentinel_password_secret_id TEXT REFERENCES secrets(id),
  tls              INTEGER NOT NULL DEFAULT 0 CHECK (tls IN (0,1)),
  tls_server_name  TEXT,
  write_mode       TEXT NOT NULL DEFAULT 'confirm' CHECK (write_mode IN ('read_only','confirm','open')),
  default_db       INTEGER NOT NULL DEFAULT 0,
  key_separator    TEXT NOT NULL DEFAULT ':',
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL,
  last_opened_at   INTEGER,
  CHECK ((route = 'host') = (host_id IS NOT NULL))
);
CREATE INDEX redis_databases_vault_idx ON redis_databases(vault_id, name);
CREATE INDEX redis_databases_host_idx ON redis_databases(host_id);

-- Console history, encrypted with the vault's data key like command_history after 0005. 500 rows per database.
CREATE TABLE redis_history (
  id           TEXT PRIMARY KEY,
  database_id  TEXT NOT NULL REFERENCES redis_databases(id) ON DELETE CASCADE,
  command_enc  BLOB NOT NULL,
  ran_at       INTEGER NOT NULL
);
CREATE INDEX redis_history_db_idx ON redis_history(database_id, ran_at DESC);
