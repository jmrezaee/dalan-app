# Restore any dump, and copy between databases: spec

Extends `replica/databases/spec.md` (read it first; everything there about routes, secrets, the gate, modes, production and the transfer tab applies). Builds on what D6, D7 and D10 shipped: `sql/backup.rs` (dump and restore tools run on the host over SSH, password never on a command line), `sql/transfer.rs`, `sql/mytransfer.rs`, `mongo/docs.rs`, the Transfer tabs (`SqlTransferTab`, `MongoTransfer`).

Asked for by Mehrdad on 2026-11-27:

1. **Restore from a single file made anywhere,** not only by Dalan's own backup: detect the format, show what is inside, restore into a chosen target behind the same gate.
2. **Copy straight from one database to another,** same engine or across engines where it makes sense, schema plus data, with table selection, type mapping, progress and Stop.

## 1. Screens and flows

| ID | screen | where |
| --- | --- | --- |
| S48 | Restore a dump: pick a file, what's inside, target and options, review, progress | Transfer tab, "Restore…"; also the database list's menu and drag-and-drop of a file onto a database |
| S49 | Copy data: source and target, objects, mapping, options, review, progress, report | new inner tab "Copy" opened from the tree ("Copy to…" on a database, schema or table), the database list menu and the palette |

| ID | flow |
| --- | --- |
| F31 | Restore a dump made elsewhere into an existing or a new database |
| F32 | Copy tables or collections from one database to another (same or other engine) |

Both screens are new boards: add them to the "Databases" page of the design canvas and to `replica/design/components.md` (with every state) before building, as the spec's rule says.

## 2. Restore from any dump (S48, F31)

### Formats we accept

Detected from the first bytes, never from the file name alone (the name is only a hint shown next to the result):

| engine | format | detected by | restored with |
| --- | --- | --- | --- |
| PostgreSQL | custom (`pg_dump -Fc`) | `PGDMP` magic | `pg_restore` |
| PostgreSQL | tar (`-Ft`) | `ustar` header with `toc.dat` | `pg_restore` |
| PostgreSQL | directory (`-Fd`), picked as a folder or a `.zip`/`.tar` of it | `toc.dat` in the folder | `pg_restore` (uploaded to the host first, see below) |
| PostgreSQL | plain SQL (`-Fp`, `pg_dumpall` too) | `-- PostgreSQL database dump` header, else the statement scanner | `psql` |
| MySQL / MariaDB | `mysqldump` / `mariadb-dump` SQL | `-- MySQL dump` / `-- MariaDB dump` header, else the scanner | `mysql` / `mariadb` |
| MongoDB | `mongodump --archive` (gzipped or not) | archive magic `0x8199e26d` | `mongorestore --archive` |
| MongoDB | `mongodump` folder of `.bson` + `.metadata.json` (or a `.zip`/`.tar` of it) | the file pair layout | `mongorestore --dir` (uploaded first) |
| any SQL | generic `.sql` from another tool | the scanner guesses the dialect (quotes, `COPY … FROM stdin`, backticks, `ENGINE=`) and says it is a guess | the engine's client |

Outer compression is unwrapped in Rust while streaming: gzip, zstd, bzip2, xz, and a single-file zip. Anything else gets "Dalan doesn't recognise this file" with the first bytes it saw.

### What's inside (preview)

Rust reads the file once, streaming, without loading it (multi-GB files are normal) and shows:

- **Source:** engine and version that made it, tool version, source database name, date, encoding, compression, size. A dump newer than the target's tool says so before anything runs ("This dump came from pg_dump 17; the host has pg_restore 15, which can't read it.").
- **Objects:** schemas, tables (with a row count: exact for COPY blocks and archives, "about N" for INSERTs), views, functions and procedures, triggers, sequences, types and enums, extensions, roles and grants; for Mongo, databases, collections with document counts, indexes, views.
- **Things that will need attention,** each with what Dalan will do about it:
  - owners and grants for roles the target doesn't have (default: skip ownership and privileges, `--no-owner --no-privileges` / `--no-acl`);
  - extensions the target can't install (checked against `pg_available_extensions`);
  - `CREATE DATABASE`, `\connect`, `USE` lines naming another database (default: ignored, so everything lands in the chosen target; see "Safety");
  - MySQL `DEFINER=` clauses (default: rewritten to the current user, or removed for views/routines, since a non-SUPER user can't restore them);
  - `pg_dumpall` globals (roles, tablespaces): skipped unless asked, and then typed.
- How the custom and tar formats are read: our own reader for the archive header and table of contents (format version, compression, entries with tag, kind, schema, owner, dependencies), unit-tested and fuzzed. If it can't read a version, fall back to `pg_restore --list` on the host or here, fed the file.
- Plain SQL and MySQL dumps: the dialect tokenizers from `sql/classify.rs` (already handle quotes, dollar quotes, backticks, `/*! */`, `DELIMITER`) drive a scanner that records object statements, COPY/INSERT targets and client commands.
- Mongo archive: the prelude (header plus one metadata document per collection) is BSON we already parse.

### Choosing what and where

- **Target:** any saved database of the same engine (MySQL and MariaDB dumps go to either, with a note on version differences), or **"A new database"** on a saved server (Dalan runs `CREATE DATABASE` first, with the dump's encoding and collation when given). A new database is the default when the target server is production.
- **Objects:** tick schemas, tables and collections from the preview (custom/tar: a `-L` list; plain SQL and MySQL: Dalan filters the stream by object as it sends it; Mongo: `--nsInclude`). "Schema only" and "Data only" switches.
- **Rename on the way:** PostgreSQL schema → another schema (plain SQL and custom by rewriting the `search_path`/qualified names in the stream for plain, `pg_restore` then `ALTER SCHEMA … RENAME` for archives); Mongo `--nsFrom/--nsTo` from the dump's database to the target's.
- **Existing objects:** "Stop if anything already exists" (default), "Drop and replace what the dump contains" (`--clean --if-exists`, `DROP TABLE IF EXISTS`, `mongorestore --drop`: typed), "Add rows to existing tables" (data only).
- **Errors:** PostgreSQL in one transaction that stops at the first error (default, so a failure leaves the database as it was); "keep going and report" as an option. MySQL and MariaDB can't undo DDL: the review says so and offers the new-database target.
- **Speed:** parallel jobs for custom and directory formats (`-j`, only with a file on the host, so it implies an upload); `--disable-triggers` for data-only restores, with the note that it needs a superuser.

### Where the tool runs, and getting the file there

- Single-file formats stream from this computer into the tool's standard input over the SSH exec channel, after the password lines the wrappers already read. Nothing is written on the host.
- Directory formats, and custom with parallel jobs, need the files on the host: Dalan uploads them over SFTP into a private `mktemp -d` folder (mode 700), runs the tool, and removes the folder when the tool ends, fails or is stopped. If the host's free space (from `df` on that folder) is under the dump size, it says so and doesn't start.
- Direct routes run the tool here, as today. A missing tool is `DB_TOOL_MISSING` naming the tool, the version needed and where.

### Safety (this is a file from outside)

A plain SQL dump run through `psql` or `mysql` on the host can do more than SQL: both clients have commands that run programs or read and write files **on the host** (`\!`, `\o |cmd`, `\g |cmd`, `\copy … program`, `\i`, `\setenv`; `system`, `\!`, `source`, `tee`, `pager`). So:

- The scanner reads every client command in the file before anything runs. Only the ones dumps actually need pass: `\connect`/`USE` to the source database (rewritten or dropped, see above), `\.` ending COPY data, `\restrict`/`\unrestrict` (new pg_dump), `\set ON_ERROR_STOP`, `\encoding`, `SET` statements, MySQL's `DELIMITER` and `/*!…*/` version comments. Any other client command refuses the restore and names the line ("Line 18,204 runs a program on the host (`\! …`). Dalan won't restore this file.").
- Dalan sends the file through itself (it is the stream into stdin), so what it scanned is what the tool gets.
- `mysql` runs with `--system-command=OFF` where the client supports it, `--binary-mode` off, `--local-infile=0`. MariaDB's `/*M!999999\- enable the sandbox mode */` header is passed through as is (it turns the sandbox on).
- Statements that reach outside the database are refused the same way: `COPY … TO/FROM PROGRAM`, `COPY … TO/FROM '<server file>'`, `LOAD DATA`, `SELECT … INTO OUTFILE/DUMPFILE`, `CREATE … LANGUAGE` untrusted, `ALTER SYSTEM`, `CREATE/ALTER ROLE … SUPERUSER`. pg_dumpall role statements need the typed "include roles" option.
- The gate is the one restore already uses: Read-only refuses; restore into an existing database is always typed with the target's name and environment shown twice; production refuses unless switched to Open with the master password. A new database is a confirm, not typed, except on production servers, where it is typed too.
- The review lists, in plain words: target, what will be dropped or replaced, object and row counts, what will be skipped (owners, roles, DEFINER, extensions), and where the tool runs.

### Progress, Stop and the result

- A progress bar by bytes read from the file, plus the current object (from the archive's TOC as `pg_restore --verbose` reports it, or from the scanner's position in a plain file), rows per second and time left.
- Stop kills the tool (and the remote process group) and removes any uploaded folder. Restore in one transaction: "Nothing changed." Otherwise: "Stopped after N objects; these are in the database now", with the list.
- The result screen: objects restored, warnings from the tool's stderr (deduplicated: "role app_owner does not exist ×214"), time taken, and "Open the database". The log is kept with the transfer history.

## 3. Copy between databases (S49, F32)

### Pairs

| source → target | schema | data |
| --- | --- | --- |
| PostgreSQL → PostgreSQL | tables, columns, keys, indexes, constraints, sequences and identities, enums and domains, views (in dependency order); functions, procedures and triggers opt-in | binary `COPY` when both are the same major version, text `COPY` otherwise |
| MySQL/MariaDB → MySQL/MariaDB | tables (`SHOW CREATE TABLE`), indexes, keys, views, triggers and routines opt-in | streamed rows into batched multi-row `INSERT` (500 rows or 1 MB) |
| PostgreSQL ↔ MySQL/MariaDB | tables, columns, primary and unique keys, indexes, foreign keys, defaults that translate, auto-increment ↔ identity; views, functions, triggers are listed as not copied, with their DDL to copy by hand | streamed, converted per column |
| MongoDB → MongoDB | collections with options (capped, collation, time series, clustered), indexes, validators, views | `find` batches into ordered `insert` batches, exact BSON types |
| SQL → MongoDB, MongoDB → SQL | not in this version (listed in §6) | |

The data always flows through Dalan (source route → this computer → target route), so the two databases can be on different hosts and never need to reach each other. Same-server PostgreSQL copies of a whole database can use `CREATE DATABASE … TEMPLATE` instead when nobody is connected to the source; the screen offers it when it applies.

### Type mapping (PostgreSQL ↔ MySQL/MariaDB)

A table in `sql/typemap.rs`, unit-tested in both directions. Each column shows its mapping, and a lossy one is marked with the reason:

| PostgreSQL | MySQL / MariaDB | note |
| --- | --- | --- |
| `smallint`/`integer`/`bigint` | same | |
| identity / `serial` | `AUTO_INCREMENT` | next value carried over |
| `numeric(p,s)` | `decimal(p,s)` | unbounded `numeric` → `decimal(65,30)`, marked lossy |
| `real`/`double precision` | `float`/`double` | |
| `boolean` | `tinyint(1)` | |
| `text`/`varchar(n)` | `longtext`/`varchar(n)` | a `varchar` key over 3,072 bytes becomes a prefix index, marked |
| `bytea` | `longblob` | |
| `timestamptz` | `datetime(6)` in UTC | marked: the zone is gone |
| `timestamp`/`date`/`time` | `datetime(6)`/`date`/`time(6)` | values outside MySQL's range refused per row, reported |
| `interval` | `varchar` (ISO 8601) | lossy |
| `uuid` | `char(36)` (or `binary(16)` by option) | |
| `json`/`jsonb` | `json` | |
| arrays, composite, range, `hstore` | `json` | lossy |
| enum type | `enum(…)` | |
| geometry (PostGIS) | geometry | only when both sides have it; else refused for that column |
| `unsigned` ints (MySQL → PG) | next wider int; `bigint unsigned` → `numeric(20)` | |
| `tinyint(1)` → `boolean`; `datetime` → `timestamp`; `set(…)` → `text[]`; `year` → `smallint`; zero dates → NULL (marked) | | |

Every mapping can be overridden per column in the mapping step, and the generated DDL is shown and can be edited before the run (it is checked again by the classifier when it is).

### Choosing

1. **Source and target:** any two saved SQL databases, or two MongoDB databases. The target can be "A new database" on a saved server, as in restore. Source and target the same database is allowed only with a different schema or table name (a copy within the database).
2. **Objects:** a tree with checkboxes (schemas, tables, views, sequences, routines; collections), search, "select related" (follows foreign keys so a selection doesn't break them), rename per object, and per table: a filter (one condition, checked like the grid's), a row limit (for a sample), columns to leave out.
3. **Existing tables at the target:** Create (stop if it exists, default), Drop and recreate (typed), Empty then load (`TRUNCATE`, typed), Append, Upsert by primary key (`ON CONFLICT … DO UPDATE` / `ON DUPLICATE KEY UPDATE`; Mongo `replaceOne` with upsert).
4. **Options:** schema only / data only / both; copy indexes and foreign keys after the data (default, faster and order-free); stop at the first error (default) or skip the failing table and go on; batch size.

### Running it

- The source is read in one consistent snapshot: `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY` (PostgreSQL), `START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY` (InnoDB), a `snapshot` read concern where the Mongo server supports it (otherwise it says the copy isn't a single point in time).
- Order: create schemas, types and tables without foreign keys → load each table (one target transaction per table where the engine allows) → indexes, unique keys and foreign keys → sequences and auto-increment set past the highest copied value → views and the opt-in routines.
- Streaming with backpressure: the source cursor reads a batch only when the target has taken the previous one; memory stays inside `test-plan.md`'s budget for any table size.
- Progress per table (rows done of the estimate, rows per second, time left) and overall; Stop finishes the current batch, rolls back the current table's transaction, and reports which tables are complete. "Run again from the failed table" resumes.
- The gate: the target's write is one held `Pending` listing everything (tables created, dropped, emptied, upserted; row estimates). Read-only target refuses. Confirm level for create-only into a new or empty target; typed when anything existing is dropped, emptied or upserted, or the target is production (and production needs Open). The source only needs reading, so a production source is fine.
- The report: per table rows copied, rows refused with the first reasons, time; columns that were mapped lossily; objects not copied with their DDL. A row count check (source snapshot vs. target) runs at the end and is shown per table.

## 4. Rust

- `src-tauri/src/dbs/dumpfile.rs`: format detection, decompression stream, the PostgreSQL archive TOC reader, the Mongo archive prelude reader, the plain-SQL scanner (preview, client-command check, object filter, database-name rewrite), unit tests and a `cargo fuzz` target per reader.
- `sql/backup.rs`: `psql` as a restore tool for plain PostgreSQL SQL; `--nsFrom/--nsTo`, `--dir`, `-j`, `-L` args; the upload-then-run path over SFTP with cleanup; the `mysql` safety flags.
- `src-tauri/src/dbs/copy.rs`: the job (plan, run, stop, resume, report); `sql/typemap.rs`; per-engine readers and writers reusing `sql/run.rs`'s cursor, `MyConn::stream`, `COPY` from `sql/transfer.rs`, and the Mongo find/insert in `mongo/docs.rs`; the DDL generators reusing `sql/structure.rs` and `sql/myextra.rs`.
- Commands: `restore_pick(path?) -> DumpInfo` (the path stays in Rust, the window gets an id), `restore_plan(dump_id, target, options) -> RestorePlan`, `restore_prepare(plan) -> Pending`, `restore_commit(pending_id, typed_name?)` with progress on a `Channel`, `restore_stop`; `copy_plan(source, target, selection, options) -> CopyPlan` (DDL, mappings, warnings, estimates), `copy_prepare(plan) -> Pending`, `copy_commit(pending_id, typed_name?, progress: Channel)`, `copy_stop`, `copy_resume`. Typed via tauri-specta.
- New error codes: `DB_DUMP_UNKNOWN`, `DB_DUMP_TOO_NEW`, `DB_DUMP_UNSAFE` (names the line), `DB_HOST_NO_SPACE`, `SQL_TYPE_UNMAPPED`, `DB_COPY_PARTIAL`.
- New crates (small, pure Rust): `async-compression` (gzip, zstd, bzip2, xz on tokio streams) and `zip` (reading a zipped directory dump). Nothing else.
- No schema change is needed for this part: job history goes in the existing `db_history` with kinds `restore` and `copy`.

## 5. UI

- `app/components/databases/RestoreDialog.vue` (S48): a stepper (File → What's inside → Target and options → Review → Progress → Result); drop a file anywhere on a database tab or on the list.
- `app/components/databases/CopyTab.vue` (S49): two route cards side by side (source, target, with engine badge and environment chip), the object tree, the mapping table (per column: source type, target type, lossy marker, override), DDL preview in CodeMirror, Review, live progress table, Report.
- Stores: `useRestoreStore(id)`, `useCopyStore(tabKey)`; both reach Rust only through `useTauri` wrappers.
- Copy (our voice): "Restore into a new database instead" as the suggested safe path; "Lossy: the time zone is dropped, values are stored in UTC."; "Nothing changed." after a stopped single-transaction restore.

## 6. Not in this version

SQL ↔ MongoDB copies (rows to documents and back), SQL Server and SQLite as sources or targets, Oracle and other dump formats, restoring physical backups (`pg_basebackup`, Percona XtraBackup, file-system snapshots), point-in-time recovery from WAL or binlogs, continuous replication or change-data-capture sync.

## 7. Tests

- **Unit:** format detection on a corpus of real dumps made in Docker (`tests/fixtures/dumps/`: pg_dump 13–17 in all four formats, pg_dumpall, mysqldump 8.0/8.4, mariadb-dump 11, mongodump archive and folder, each gzipped and zstd'd, plus truncated and hostile files); TOC and prelude readers; the scanner's client-command check (every refused command, in both clients, hidden in comments, strings, dollar quotes and COPY data where they are harmless); database-name rewrite; type mapping both ways; DDL generation per dialect.
- **Fuzz:** each reader for 10 minutes in CI nightly.
- **Docker (`l3_`):** restore each fixture into a new and an existing database through the sshd host and directly; the stop paths; a dump too new for the tool; a hostile `\!` file is refused before anything runs; copies PG→PG, MySQL→MariaDB, PG→MySQL and MySQL→PG of the seeded `app` schema with a row-count and checksum comparison; Mongo→Mongo of `shop` with exact types; Stop and resume mid-table.
- **Browser:** S48 and S49 flows on the fake backend, typed confirmation stays disabled until the name matches, Cancel is focused, a11y for each step, visual baselines.
- **Hand checks:** section K in `replica/testing/hand-checks.md`: a real dump from a team server restored into a new database through the bastion; a copy from staging Postgres into a local one; a cross-engine copy of a real MySQL schema.

## 8. Phases

- **D12 Restore any dump (S48, F31):** `dbs/dumpfile.rs` with its fixtures and fuzz targets, the safety scanner, `psql` and the upload path, the new restore commands and `RestoreDialog`, for all four engines. Hand check K1.
- **D13 Copy between databases (S49, F32):** same-engine first (PG→PG, MySQL/MariaDB→MySQL/MariaDB, Mongo→Mongo), then PG ↔ MySQL/MariaDB with `typemap.rs`; plan, gate, run, stop, resume, report; `CopyTab`. Hand checks K2 and K3.

## 9. Extras (agreed with Mehrdad on 2026-11-27)

All eight extras were accepted. Each gets its own phase after D13, in this order: the ones that reuse the copy engine first, scheduling last because it depends on saved jobs.

### E1 Saved jobs (D14)

- Any restore, copy, backup or bulk transfer set up in S48/S49/S43 can be saved by name ("Nightly staging refresh") and rerun with one click from the database list menu, the palette, or a new **Jobs** entry in the vault nav (Lucide `ListChecks`).
- A job stores its plan (source and target database ids, selection, options, masking rules), never a password: credentials come from the vault when it runs.
- Running a saved job goes through the gate exactly like the first run: the held `Pending` is built fresh from the current state (table counts, whether targets exist), so nothing approved earlier carries over. A job whose target is production always asks for the typed name, even when it is unattended (so production targets can't be scheduled, see E2).
- Each run is recorded: start, end, result, rows, warnings, the log (kept for the last 50 runs per job).

### E2 Scheduled backups with retention (D15)

- A saved backup job (and a copy job whose target is not production) can have a schedule: hourly, daily at a time, weekly on a day, or a cron expression, in local time.
- Runs only while Dalan is open **and unlocked**, because the passwords come from the vault. A run that was due while Dalan was closed or locked runs once at the next unlock (not once per missed slot), and the Jobs list says "Missed 3 runs while locked; ran at 09:12".
- Retention per job: keep the last N, or N days, or both; old files are removed only from the job's own folder and only files whose name matches the job's pattern (`<db>-<job>-<yyyymmdd-hhmm>.<ext>`).
- A notification when a run fails (system notification plus a dot on Jobs); nothing on success unless turned on.
- Destinations: a folder on this computer or a folder on a host (E8).

### E3 Schema compare and sync (D16)

- Compare two databases or schemas of the same engine (PostgreSQL to PostgreSQL, MySQL/MariaDB to MySQL/MariaDB), or a database with a dump file (D12's reader), or with a saved snapshot of a schema.
- The result is a tree of differences: tables, columns, types, defaults, nullability, keys, indexes, foreign keys, views, functions, triggers, sequences, enums; each marked added, removed or changed, with the two DDLs side by side.
- "Sync" turns the ticked differences into one migration script in dependency order (drops last and marked), shown in the editor, then run through the gate like any structure change (drops and type changes typed). "Copy as migration" saves it to a file instead.
- Reuses `sql/structure.rs` and `sql/myextra.rs` for the catalog; the differ is `dbs/schemadiff.rs`, unit-tested on pairs of catalogs.

### E4 Data compare (D16)

- Compare two tables (same or different databases, same engine or across PG ↔ MySQL through the type map) by primary key, or by chosen key columns.
- Streams both sides sorted by key and merges them (constant memory): only in source, only in target, different (with the changed columns), identical (count only). For big tables, an optional quick pass by hashed key ranges first, drilling into the ranges that differ.
- "Make target match" builds the inserts, updates and deletes, held through the gate (deletes typed on production as always), or saves them as a script.

### E5 Masking for copies (D14)

- Rules per column, set in the copy mapping step or saved with a job: keep, null, fixed value, fake (email, name, phone, address, IBAN-like, with the same shape and length), hash (keyed per job, so the same input gives the same output across tables and foreign keys stay joined), shuffle within the column, truncate, date shift by a random number of days within a range.
- Suggested rules: Dalan proposes masking for columns whose names or sampled values look personal (`email`, `phone`, `name`, `address`, `ssn`, `card`, `iban`, `birth`), shown as suggestions, never applied silently.
- A copy from a production source to a non-production target with no masking rules shows a reminder in the review ("Copying production data to Staging with no masking. Add rules?"), not a block.
- Masking happens in Dalan's stream between source and target; unmasked values never reach the target. Also available for exports (bulk and single table).
- Fake values come from a small built-in generator seeded per job (no network, no external data set).

### E6 Bulk export and import (D17)

- Export many tables or collections at once to a folder (or a `.zip`): one file per table, CSV, TSV, JSON lines or SQL inserts, plus a `manifest.json` (tables, columns and types, row counts, the source, the time). Optional schema DDL beside the data.
- Import a folder or zip of CSV/JSON files: each file is matched to a table by name (overridable), with the D6 preview and mapping per file, "create missing tables" from inferred types (reviewed DDL), and one run with progress for all.
- **MongoDB import** (the gap left by D10): JSON lines, a JSON array, extended JSON, or CSV with typed columns into a collection, through the gate; insert, upsert by `_id` or a chosen field, or replace the collection (typed).

### E7 Backup check (D17)

- After a backup (manual or scheduled), optionally verify it: restore it into a scratch database on a chosen server (never production; the scratch database is named `dalan_check_<id>` and dropped after), then compare table and collection row counts with the source and report.
- A cheaper "read check" always runs at the end of a backup: the file is read back with D12's reader to confirm it's complete (archive TOC readable, gzip trailer valid, the dump's end marker present).

### E8 Backups to a folder on a host (D15)

- A backup's destination can be a folder on any saved host instead of this computer: the tool's output streams to the file over SFTP (when the dump runs on that same host, it is written there directly with no round trip through this computer).
- Restore (D12) and backup check (E7) can read a dump from a host folder too, with an SFTP file picker.
- Retention (E2) applies to host folders the same way.

### Data model: migration 0008

Never edit 0007. `0008_jobs.sql` (to be confirmed with Mehrdad before D14 starts):

- `db_jobs`: id, name, kind (`backup`, `restore`, `copy`, `export`, `import`, `compare`), plan (JSON, encrypted like history since it can hold filters and masking keys), schedule (null or JSON), retention (JSON), destination (local path or host id + path), created, updated.
- `db_job_runs`: id, job_id, started, ended, status (`ok`, `failed`, `stopped`, `missed`), rows, bytes, warnings count, log (encrypted, capped).
- `schema_snapshots` (for E3): id, database id, captured, catalog (JSON).
- Deleting a database or host refuses while a job uses it, naming the job (the Redis rule for routes).

### Phases (after D13)

- **D14 Saved jobs and masking (E1, E5):** migration 0008, Jobs view, save and rerun from S48/S49, masking rules in the copy stream and exports.
- **D15 Scheduled backups and host destinations (E2, E8):** the scheduler (in Rust, a tokio task started on unlock and stopped on lock), retention, missed-run handling, notifications, SFTP destination and picker.
- **D16 Schema compare and data compare (E3, E4).**
- **D17 Bulk export and import, Mongo import, backup check (E6, E7).**
- **D18 Hardening for D12 to D17:** walk chapters, visual baselines, a11y, performance (a 10M-row copy and compare inside the memory budget), hand checks K4 to K9, `features.csv`. Ends `[h]`.
