-- 0007: one table for every database engine (replica/databases/spec.md section 2). Redis rows keep their ids, so
-- settings and open-tab state that name a Redis database keep working. The two Redis tables are copied and dropped.
CREATE TABLE databases (
  id                TEXT PRIMARY KEY,
  vault_id          TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  engine            TEXT NOT NULL CHECK (engine IN ('redis','postgres','mysql','mariadb','mongodb')),
  name              TEXT NOT NULL,
  environment       TEXT NOT NULL DEFAULT 'none' CHECK (environment IN ('none','staging','production')),
  route             TEXT NOT NULL CHECK (route IN ('direct','host')),
  host_id           TEXT REFERENCES hosts(id) ON DELETE RESTRICT,  -- required when route = 'host'
  address           TEXT NOT NULL,     -- Mongo: a seed list "h1:p1,h2:p2" or an SRV name; Redis Sentinel: a comma list
  port              INTEGER,           -- NULL when address carries the ports
  default_database  TEXT,              -- Postgres/MySQL database, Mongo database, Redis db index as text
  username          TEXT,
  password_secret_id TEXT REFERENCES secrets(id),
  extra_secret_id   TEXT REFERENCES secrets(id),                    -- Redis Sentinel password; reserved for keys
  tls               TEXT NOT NULL DEFAULT 'off' CHECK (tls IN ('off','require','verify')),
  tls_server_name   TEXT,
  write_mode        TEXT NOT NULL DEFAULT 'confirm' CHECK (write_mode IN ('read_only','confirm','open')),
  options           TEXT NOT NULL DEFAULT '{}',   -- engine-specific JSON, validated in Rust per engine
  created_at        INTEGER NOT NULL,
  updated_at        INTEGER NOT NULL,
  last_opened_at    INTEGER,
  CHECK ((route = 'host') = (host_id IS NOT NULL))
);
CREATE INDEX databases_vault_idx ON databases(vault_id, engine, name);
CREATE INDEX databases_host_idx ON databases(host_id);

-- Console, SQL and pipeline history, encrypted with the vault's data key. 500 rows per database and kind.
CREATE TABLE database_history (
  id           TEXT PRIMARY KEY,
  database_id  TEXT NOT NULL REFERENCES databases(id) ON DELETE CASCADE,
  kind         TEXT NOT NULL DEFAULT 'command',   -- 'command' (Redis, Mongo shell), 'sql', 'pipeline'
  body_enc     BLOB NOT NULL,
  ran_at       INTEGER NOT NULL,
  duration_ms  INTEGER,
  rows         INTEGER,
  ok           INTEGER
);
CREATE INDEX database_history_db_idx ON database_history(database_id, ran_at DESC);

CREATE TABLE saved_queries (
  id           TEXT PRIMARY KEY,
  vault_id     TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  database_id  TEXT REFERENCES databases(id) ON DELETE SET NULL,  -- NULL: usable on any database of that engine
  engine       TEXT NOT NULL,
  kind         TEXT NOT NULL CHECK (kind IN ('sql','pipeline','filter','command')),
  folder       TEXT,
  name         TEXT NOT NULL,
  description  TEXT,
  body_enc     BLOB NOT NULL,                  -- encrypted with the data key, like history
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL
);
CREATE INDEX saved_queries_vault_idx ON saved_queries(vault_id, engine, folder, name);

INSERT INTO databases (id, vault_id, engine, name, environment, route, host_id, address, port, default_database, username,
                       password_secret_id, extra_secret_id, tls, tls_server_name, write_mode, options,
                       created_at, updated_at, last_opened_at)
SELECT id, vault_id, 'redis', name, environment, route, host_id, address, port, CAST(default_db AS TEXT), username,
       password_secret_id, sentinel_password_secret_id, CASE tls WHEN 1 THEN 'verify' ELSE 'off' END, tls_server_name, write_mode,
       json_object('kind', kind, 'sentinel_master', sentinel_master, 'key_separator', key_separator),
       created_at, updated_at, last_opened_at
FROM redis_databases;

INSERT INTO database_history (id, database_id, kind, body_enc, ran_at)
SELECT id, database_id, 'command', command_enc, ran_at FROM redis_history;

DROP TABLE redis_history;
DROP TABLE redis_databases;
