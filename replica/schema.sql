-- Termius clone: local SQLite schema, migration 0001.
-- Owned and run by the Rust core (rusqlite + rusqlite_migration). The webview never opens this file.
--
-- Conventions
--   id          TEXT uuid v7 (time-ordered), generated in Rust
--   timestamps  INTEGER unix epoch milliseconds, UTC (SQLite has no timestamptz)
--   booleans    INTEGER 0/1 with CHECK
--   secrets     never stored in plaintext; every password, passphrase and private key is a row in `secrets`,
--               encrypted with XChaCha20-Poly1305 under the vault key (Argon2id from the master password)
--   owner       single local user; `vault_id` is the owner column on everything, kept for future sync
--   access      data-layer checks in Rust: every command requires an unlocked vault; no row-level security in SQLite
--
-- Connection pragmas (set on every open, not in the migration):
--   PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;

-- ---------------------------------------------------------------- vault + secrets

CREATE TABLE vaults (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  kind          TEXT NOT NULL DEFAULT 'local' CHECK (kind IN ('local')),
  kdf_salt      BLOB NOT NULL,                 -- 16+ random bytes
  kdf_params    TEXT NOT NULL,                 -- json: {"alg":"argon2id","m":65536,"t":3,"p":1}
  key_check     BLOB NOT NULL,                 -- a known value encrypted under the vault key; verifies the master password
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

CREATE TABLE secrets (
  id            TEXT PRIMARY KEY,
  vault_id      TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  kind          TEXT NOT NULL CHECK (kind IN ('password','passphrase','private_key','env_value')),
  nonce         BLOB NOT NULL,                 -- 24 bytes
  ciphertext    BLOB NOT NULL,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);
CREATE INDEX secrets_vault_idx ON secrets(vault_id);

-- ---------------------------------------------------------------- keychain

CREATE TABLE keys (
  id                    TEXT PRIMARY KEY,
  vault_id              TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  label                 TEXT NOT NULL,
  key_type              TEXT NOT NULL CHECK (key_type IN ('ed25519','ecdsa-p256','ecdsa-p384','ecdsa-p521','rsa','sk-ed25519','sk-ecdsa')),
  bits                  INTEGER,               -- rsa only
  public_key            TEXT NOT NULL,         -- OpenSSH one-line format, safe to show and copy
  fingerprint_sha256    TEXT NOT NULL,
  private_key_secret_id TEXT REFERENCES secrets(id) ON DELETE SET NULL,
  passphrase_secret_id  TEXT REFERENCES secrets(id) ON DELETE SET NULL,  -- null = not saved, prompt on every connect
  certificate           TEXT,                  -- OpenSSH user certificate (key + cert are one item)
  created_at            INTEGER NOT NULL,
  updated_at            INTEGER NOT NULL
);
CREATE INDEX keys_vault_idx ON keys(vault_id, label);
CREATE UNIQUE INDEX keys_fingerprint_uq ON keys(vault_id, fingerprint_sha256);

CREATE TABLE identities (
  id                 TEXT PRIMARY KEY,
  vault_id           TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  label              TEXT NOT NULL,
  username           TEXT NOT NULL,
  password_secret_id TEXT REFERENCES secrets(id) ON DELETE SET NULL,
  key_id             TEXT REFERENCES keys(id) ON DELETE SET NULL,
  created_at         INTEGER NOT NULL,
  updated_at         INTEGER NOT NULL
);
CREATE INDEX identities_vault_idx ON identities(vault_id, label);
CREATE INDEX identities_key_idx ON identities(key_id);

-- ---------------------------------------------------------------- snippets (before profiles: startup snippet FK)

CREATE TABLE snippet_packages (
  id          TEXT PRIMARY KEY,
  vault_id    TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  label       TEXT NOT NULL,
  sort        INTEGER NOT NULL DEFAULT 0,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);
CREATE INDEX snippet_packages_vault_idx ON snippet_packages(vault_id, sort);

CREATE TABLE snippets (
  id          TEXT PRIMARY KEY,
  vault_id    TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  package_id  TEXT REFERENCES snippet_packages(id) ON DELETE SET NULL,  -- deleting a package keeps its snippets
  label       TEXT NOT NULL,
  script      TEXT NOT NULL,
  sort        INTEGER NOT NULL DEFAULT 0,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);
CREATE INDEX snippets_vault_idx ON snippets(vault_id, label);
CREATE INDEX snippets_package_idx ON snippets(package_id, sort);

-- ---------------------------------------------------------------- connection profiles (the inheritable settings)
-- Every host and every group owns exactly one profile. NULL in a column = inherit from the parent group, per field.
-- Resolution: host.profile -> group.profile -> parent group.profile -> ... -> app defaults (settings table).

CREATE TABLE connection_profiles (
  id                  TEXT PRIMARY KEY,
  vault_id            TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  username            TEXT,
  identity_id         TEXT REFERENCES identities(id) ON DELETE SET NULL,
  password_secret_id  TEXT REFERENCES secrets(id) ON DELETE SET NULL,
  key_id              TEXT REFERENCES keys(id) ON DELETE SET NULL,
  ssh_port            INTEGER CHECK (ssh_port BETWEEN 1 AND 65535),
  proxy_type          TEXT CHECK (proxy_type IN ('none','socks5','http')),   -- 'none' explicitly overrides an inherited proxy
  proxy_host          TEXT,
  proxy_port          INTEGER CHECK (proxy_port BETWEEN 1 AND 65535),
  proxy_username      TEXT,
  proxy_secret_id     TEXT REFERENCES secrets(id) ON DELETE SET NULL,
  chain_mode          TEXT CHECK (chain_mode IN ('inherit','none','custom')),  -- 'custom' = use profile_jump_hosts rows
  agent_forwarding    INTEGER CHECK (agent_forwarding IN (0,1)),
  startup_snippet_id  TEXT REFERENCES snippets(id) ON DELETE SET NULL,
  env_mode            TEXT CHECK (env_mode IN ('inherit','merge','replace')),
  terminal_theme      TEXT,
  font_size           INTEGER CHECK (font_size BETWEEN 6 AND 48),
  charset             TEXT,
  keepalive_secs      INTEGER CHECK (keepalive_secs >= 0),
  sftp_start_path     TEXT,
  created_at          INTEGER NOT NULL,
  updated_at          INTEGER NOT NULL
);
CREATE INDEX profiles_identity_idx ON connection_profiles(identity_id);
CREATE INDEX profiles_key_idx ON connection_profiles(key_id);
CREATE INDEX profiles_snippet_idx ON connection_profiles(startup_snippet_id);

CREATE TABLE profile_env_vars (
  profile_id       TEXT NOT NULL REFERENCES connection_profiles(id) ON DELETE CASCADE,
  name             TEXT NOT NULL CHECK (name GLOB '[A-Za-z_]*'),
  value            TEXT,                         -- plain value, or
  value_secret_id  TEXT REFERENCES secrets(id) ON DELETE SET NULL,  -- for sensitive values
  sort             INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (profile_id, name)
);

-- ---------------------------------------------------------------- groups, hosts, tags

CREATE TABLE groups (
  id          TEXT PRIMARY KEY,
  vault_id    TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  parent_id   TEXT REFERENCES groups(id) ON DELETE RESTRICT,   -- app moves children to the parent first, in one transaction
  profile_id  TEXT NOT NULL UNIQUE REFERENCES connection_profiles(id) ON DELETE RESTRICT,
  label       TEXT NOT NULL,
  sort        INTEGER NOT NULL DEFAULT 0,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  CHECK (parent_id IS NULL OR parent_id <> id)   -- deeper cycles are rejected in Rust before writing
);
CREATE INDEX groups_parent_idx ON groups(vault_id, parent_id, sort);

CREATE TABLE hosts (
  id                 TEXT PRIMARY KEY,
  vault_id           TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  group_id           TEXT REFERENCES groups(id) ON DELETE RESTRICT,
  profile_id         TEXT NOT NULL UNIQUE REFERENCES connection_profiles(id) ON DELETE RESTRICT,
  label              TEXT NOT NULL,
  address            TEXT NOT NULL,             -- hostname, IPv4 or IPv6 literal
  os_hint            TEXT,                      -- detected after connect, for the icon
  sftp_enabled       INTEGER NOT NULL DEFAULT 1 CHECK (sftp_enabled IN (0,1)),
  last_connected_at  INTEGER,
  sort               INTEGER NOT NULL DEFAULT 0,
  created_at         INTEGER NOT NULL,
  updated_at         INTEGER NOT NULL
);
CREATE INDEX hosts_group_idx ON hosts(vault_id, group_id, sort);
CREATE INDEX hosts_label_idx ON hosts(vault_id, label);
CREATE INDEX hosts_recent_idx ON hosts(vault_id, last_connected_at DESC);

-- ordered jump-host chain for a profile; a host used as a hop cannot be deleted until removed from chains
CREATE TABLE profile_jump_hosts (
  profile_id   TEXT NOT NULL REFERENCES connection_profiles(id) ON DELETE CASCADE,
  position     INTEGER NOT NULL CHECK (position >= 0),
  jump_host_id TEXT NOT NULL REFERENCES hosts(id) ON DELETE RESTRICT,
  PRIMARY KEY (profile_id, position)
);
CREATE INDEX jump_hosts_host_idx ON profile_jump_hosts(jump_host_id);

CREATE TABLE tags (
  id          TEXT PRIMARY KEY,
  vault_id    TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  name        TEXT NOT NULL COLLATE NOCASE,
  created_at  INTEGER NOT NULL
);
CREATE UNIQUE INDEX tags_name_uq ON tags(vault_id, name);

CREATE TABLE host_tags (
  host_id  TEXT NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
  tag_id   TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  PRIMARY KEY (host_id, tag_id)
);
CREATE INDEX host_tags_tag_idx ON host_tags(tag_id);

-- default target hosts remembered per snippet
CREATE TABLE snippet_targets (
  snippet_id  TEXT NOT NULL REFERENCES snippets(id) ON DELETE CASCADE,
  host_id     TEXT NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
  PRIMARY KEY (snippet_id, host_id)
);
CREATE INDEX snippet_targets_host_idx ON snippet_targets(host_id);

-- ---------------------------------------------------------------- known hosts

CREATE TABLE known_hosts (
  id                  TEXT PRIMARY KEY,
  vault_id            TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  host_pattern        TEXT NOT NULL,          -- "address:port" exactly as connected, e.g. "10.0.0.5:22", "[::1]:2222"
  key_type            TEXT NOT NULL,          -- ssh-ed25519, ecdsa-sha2-nistp256, rsa-sha2-512 ...
  public_key          TEXT NOT NULL,          -- base64 blob
  fingerprint_sha256  TEXT NOT NULL,
  source              TEXT NOT NULL DEFAULT 'tofu' CHECK (source IN ('tofu','import','manual')),
  created_at          INTEGER NOT NULL,
  last_seen_at        INTEGER
);
CREATE UNIQUE INDEX known_hosts_uq ON known_hosts(vault_id, host_pattern, key_type);

-- ---------------------------------------------------------------- port forwarding

CREATE TABLE port_forwards (
  id            TEXT PRIMARY KEY,
  vault_id      TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  host_id       TEXT NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
  label         TEXT NOT NULL,
  kind          TEXT NOT NULL CHECK (kind IN ('local','remote','dynamic')),
  bind_address  TEXT NOT NULL DEFAULT '127.0.0.1',
  bind_port     INTEGER NOT NULL CHECK (bind_port BETWEEN 1 AND 65535),
  dest_address  TEXT,
  dest_port     INTEGER CHECK (dest_port BETWEEN 1 AND 65535),
  autostart     INTEGER NOT NULL DEFAULT 0 CHECK (autostart IN (0,1)),
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  CHECK ((kind = 'dynamic' AND dest_address IS NULL AND dest_port IS NULL)
      OR (kind <> 'dynamic' AND dest_address IS NOT NULL AND dest_port IS NOT NULL))
);
CREATE INDEX port_forwards_host_idx ON port_forwards(host_id);
-- two local/dynamic rules cannot listen on the same local address:port
CREATE UNIQUE INDEX port_forwards_local_bind_uq ON port_forwards(vault_id, bind_address, bind_port) WHERE kind IN ('local','dynamic');

-- ---------------------------------------------------------------- workspaces, history, logs

CREATE TABLE workspace_templates (
  id          TEXT PRIMARY KEY,
  vault_id    TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  mode        TEXT NOT NULL DEFAULT 'split' CHECK (mode IN ('split','focus')),
  layout      TEXT NOT NULL,   -- json split tree: {"dir":"row","sizes":[50,50],"children":[{"pane":0},{"pane":1}]}
  panes       TEXT NOT NULL,   -- json array: [{"host_id":"..."|null(local),"title":"...","cwd":null,"command":null}]
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);
CREATE INDEX workspace_templates_vault_idx ON workspace_templates(vault_id, name);

CREATE TABLE connection_history (
  id            TEXT PRIMARY KEY,
  vault_id      TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  host_id       TEXT REFERENCES hosts(id) ON DELETE CASCADE,   -- null = local terminal
  connected_at  INTEGER NOT NULL
);
CREATE INDEX connection_history_recent_idx ON connection_history(vault_id, connected_at DESC);
CREATE INDEX connection_history_host_idx ON connection_history(host_id);

CREATE TABLE command_history (
  id        TEXT PRIMARY KEY,
  host_id   TEXT REFERENCES hosts(id) ON DELETE CASCADE,       -- null = local terminal
  command   TEXT NOT NULL,
  ran_at    INTEGER NOT NULL
);
CREATE INDEX command_history_host_idx ON command_history(host_id, ran_at DESC);

CREATE TABLE session_logs (
  id           TEXT PRIMARY KEY,
  vault_id     TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  host_id      TEXT REFERENCES hosts(id) ON DELETE CASCADE,    -- null = local terminal
  started_at   INTEGER NOT NULL,
  ended_at     INTEGER,
  file_name    TEXT NOT NULL,         -- encrypted log in <app_data>/logs/, chunked, same vault key
  size_bytes   INTEGER NOT NULL DEFAULT 0,
  bookmarked   INTEGER NOT NULL DEFAULT 0 CHECK (bookmarked IN (0,1)),
  comment      TEXT
);
CREATE INDEX session_logs_host_idx ON session_logs(host_id, started_at DESC);
CREATE INDEX session_logs_list_idx ON session_logs(vault_id, bookmarked, started_at DESC);
-- retention: one non-bookmarked ("recent") log per host; Rust deletes the previous one (row + file) when a session ends

-- ---------------------------------------------------------------- settings

CREATE TABLE settings (
  key         TEXT PRIMARY KEY,      -- 'appearance', 'terminal', 'shortcuts', 'sftp', 'security', 'logs', 'defaults'
  value       TEXT NOT NULL,         -- json
  updated_at  INTEGER NOT NULL
);
