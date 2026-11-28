-- 0008: saved jobs and their runs (D14, E1), and schema snapshots (E3). replica/databases/restore-copy.md §9.
-- A job's plan (what it restores or copies, filters, masking rules and key, a file or folder path) is JSON encrypted
-- with the vault's data key, the row id as associated data, like history. It never holds a password: credentials come
-- from the vault when it runs. The databases a job reads and writes are columns, so deleting one is refused while a
-- job uses it (the app names the job first).
CREATE TABLE db_jobs (
  id                  TEXT PRIMARY KEY,
  vault_id            TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  name                TEXT NOT NULL,
  kind                TEXT NOT NULL CHECK (kind IN ('backup','restore','copy','export','import','compare')),
  source_database_id  TEXT REFERENCES databases(id) ON DELETE RESTRICT,
  target_database_id  TEXT REFERENCES databases(id) ON DELETE RESTRICT,
  plan_enc            BLOB NOT NULL,
  schedule            TEXT,             -- JSON, D15
  retention           TEXT,             -- JSON, D15
  destination         TEXT,             -- JSON, D15
  created_at          INTEGER NOT NULL,
  updated_at          INTEGER NOT NULL,
  last_run_at         INTEGER
);
CREATE INDEX db_jobs_vault_idx ON db_jobs(vault_id, name);
CREATE INDEX db_jobs_source_idx ON db_jobs(source_database_id);
CREATE INDEX db_jobs_target_idx ON db_jobs(target_database_id);

-- The last 50 runs of each job.
CREATE TABLE db_job_runs (
  id          TEXT PRIMARY KEY,
  job_id      TEXT NOT NULL REFERENCES db_jobs(id) ON DELETE CASCADE,
  started_at  INTEGER NOT NULL,
  ended_at    INTEGER,
  status      TEXT NOT NULL CHECK (status IN ('ok','failed','stopped','missed')),
  rows        INTEGER,
  bytes       INTEGER,
  warnings    INTEGER NOT NULL DEFAULT 0,
  log_enc     BLOB
);
CREATE INDEX db_job_runs_job_idx ON db_job_runs(job_id, started_at DESC);

-- A database's catalog at one moment, to compare against later (E3).
CREATE TABLE schema_snapshots (
  id           TEXT PRIMARY KEY,
  database_id  TEXT NOT NULL REFERENCES databases(id) ON DELETE CASCADE,
  captured_at  INTEGER NOT NULL,
  catalog_enc  BLOB NOT NULL
);
CREATE INDEX schema_snapshots_db_idx ON schema_snapshots(database_id, captured_at DESC);
