# SQL and MongoDB in Dalan: spec

This adds PostgreSQL, MySQL, MariaDB and MongoDB to the Databases section that Redis started. It is a tool for developers who work on real servers every day: browse, query, edit rows and documents safely, change schemas, diagnose slow queries and locks, import, export and back up.

Decisions (Mehrdad, 2026-11-19 to 2026-11-22):

- **One Databases feature, many engines.** Redis becomes the first engine. A new migration moves its saved connections into a shared `databases` table; every engine reuses the route (direct or through a host), the vault secrets, the environment marker, the write modes and the two-step confirm.
- **Engines:** PostgreSQL (first), MySQL and MariaDB, MongoDB. SQLite files and SQL Server are in the backlog; the engine picker hides them until they exist.
- **Depth:** everything on the boards. This is meant to replace a commercial database client for the team.

**Design:** the "Databases" page of the UI canvas (https://claude.ai/artifact/U8iEwGC82GSXeTKYXdzanJ). Top row: the Redis boards (already built). Middle: `DbList`, `SqlData`, `SqlQuery`, `SqlStructure`, `SqlExplain`, `SqlDiagram`, `SqlActivity`, `SqlSafety`, `DbTransfer`. Bottom: `MongoDocs`, `MongoAggregate`, `MongoSchema`, `MongoShell`. Match layout and copy, with tokens, never the boards' inline hex.

**Read first:** `replica/redis/spec.md`. Everything there about transport, secrets, the gate's prepare/commit, modes, production, lock and history applies here unless this spec says otherwise. The Redis code in `src-tauri/src/redis/` is the pattern to follow.

**Clean room:** the reference for "what a pro database client does" is Navicat (with DataGrip, DBeaver and MongoDB Compass as general reference); they are named only inside `replica/`. Do not fetch, read or copy their bundles, icons, copy or layouts. Public engine documentation is fine for behaviour. Engine names (PostgreSQL, MySQL, MariaDB, MongoDB) appear in the UI as plain descriptions; no vendor logos (the engine badge is our two-letter label).

## 1. Screens and flows

| ID | screen | board |
| --- | --- | --- |
| S34 | Databases, all engines: list with engine badge, engine filter chips | DbList |
| S35 | New database: engine picker, then the engine's form (S27 generalised); paste a connection string | DbList |
| S36 | SQL tab, table Data view: filter (SQL where), sort, columns, paging, inline edits pending until commit, cell panel (JSON pretty), linked rows | SqlData |
| S37 | SQL query editor: run current / selection / all, explain, format, limit, timeout, autocomplete, several result sets, messages, history, transaction bar | SqlQuery |
| S38 | Table structure: columns, indexes, foreign keys, triggers, DDL; edits become a reviewed `alter` | SqlStructure |
| S39 | Explain: plan tree with time, estimate vs actual, a plain-language hint; history and saved queries side panel | SqlExplain |
| S40 | Diagram: tables and foreign keys, N steps out from a table, export SVG/PNG | SqlDiagram |
| S41 | Activity: tiles, sessions, blocking chains, cancel query, end session; tabs for slow queries, tables, roles, replication | SqlActivity |
| S42 | SQL gate: commit review, no-where warning, typed dangerous, read-only, open transaction on close, the classes | SqlSafety |
| S43 | Import, export and backup (all engines) | DbTransfer |
| S44 | MongoDB documents: filter, project, sort, limit, collation; list, table and JSON views; document editor | MongoDocs |
| S45 | Aggregation builder: stages on/off, output after each stage, hint, export code, create view | MongoAggregate |
| S46 | Schema sample, indexes with use, validation rules | MongoSchema |
| S47 | MongoDB shell and its gate | MongoShell |

| ID | flow |
| --- | --- |
| F20 | Add a database of any engine (form or connection string), test it, open it |
| F21 | Browse a SQL table: filter, sort, page, follow foreign keys |
| F22 | Edit rows in the grid and commit them in one transaction |
| F23 | Write and run queries; work inside a transaction; save and share queries |
| F24 | Change a table's structure through a reviewed statement |
| F25 | Diagnose: explain a query, find blocking sessions, cancel or end one |
| F26 | Import, export, back up and restore |
| F27 | Browse, filter and edit MongoDB documents |
| F28 | Build and run an aggregation pipeline |
| F29 | Inspect schema, indexes and validation of a collection |
| F30 | Use the MongoDB shell |

Shell changes:

- Vault nav: "Databases" now counts every engine. New entry **Saved queries** (Lucide `Star`) lists saved SQL queries and Mongo pipelines across databases.
- Tabs: engine badge instead of the Redis icon on every database tab. Production keeps the amber top line and chip.
- A SQL or Mongo tab has its own inner tab strip (tables, query editors, explain, diagram; collections) that stays mounted like everything else (hard rule 4 applies to editors with unsaved text: never unmount them while the tab lives).
- Palette: databases, and inside an open database its tables, collections and saved queries ("Open orders in app-prod").
- Host menu: "Open a database on this host…" lists the host's saved databases of every engine and "New database here".

## 2. Data model: migration 0007

Never edit 0006. `0007_databases.sql`:

```sql
CREATE TABLE databases (
  id TEXT PRIMARY KEY,                          -- the Redis rows keep their ids
  vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  engine TEXT NOT NULL CHECK (engine IN ('redis','postgres','mysql','mariadb','mongodb')),
  name TEXT NOT NULL,
  environment TEXT NOT NULL DEFAULT 'none' CHECK (environment IN ('none','staging','production')),
  route TEXT NOT NULL CHECK (route IN ('direct','host')),
  host_id TEXT REFERENCES hosts(id) ON DELETE RESTRICT,
  address TEXT NOT NULL,                        -- Mongo: a seed list "h1:p1,h2:p2" or an SRV name
  port INTEGER,                                 -- NULL when address carries ports (Mongo seed list, SRV)
  default_database TEXT,                        -- Postgres/MySQL database, Mongo database, Redis db index as text
  username TEXT,
  password_secret_id TEXT REFERENCES secrets(id),
  extra_secret_id TEXT REFERENCES secrets(id),  -- Redis Sentinel password; reserved for client-cert keys
  tls TEXT NOT NULL DEFAULT 'off' CHECK (tls IN ('off','require','verify')),
  tls_server_name TEXT,
  write_mode TEXT NOT NULL DEFAULT 'confirm' CHECK (write_mode IN ('read_only','confirm','open')),
  options TEXT NOT NULL DEFAULT '{}',           -- engine-specific JSON, validated in Rust per engine (section 3)
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, last_opened_at INTEGER,
  CHECK ((route = 'host') = (host_id IS NOT NULL))
);
CREATE INDEX databases_vault_idx ON databases(vault_id, engine, name);
CREATE INDEX databases_host_idx ON databases(host_id);

CREATE TABLE database_history (                -- replaces redis_history; same encryption
  id TEXT PRIMARY KEY,
  database_id TEXT NOT NULL REFERENCES databases(id) ON DELETE CASCADE,
  kind TEXT NOT NULL DEFAULT 'command',        -- 'command' (Redis, Mongo shell), 'sql', 'pipeline'
  body_enc BLOB NOT NULL,
  ran_at INTEGER NOT NULL,
  duration_ms INTEGER, rows INTEGER, ok INTEGER
);
CREATE INDEX database_history_db_idx ON database_history(database_id, ran_at DESC);

CREATE TABLE saved_queries (
  id TEXT PRIMARY KEY,
  vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  database_id TEXT REFERENCES databases(id) ON DELETE SET NULL,  -- NULL: usable on any database of that engine
  engine TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('sql','pipeline','filter','command')),
  folder TEXT, name TEXT NOT NULL, description TEXT,
  body_enc BLOB NOT NULL,                       -- encrypted with the data key like history
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE INDEX saved_queries_vault_idx ON saved_queries(vault_id, engine, folder, name);
```

Then copy `redis_databases` into `databases` (engine `redis`; `kind`, `sentinel_master`, `default_db`, `key_separator` into `options`; `sentinel_password_secret_id` into `extra_secret_id`; `tls` 0/1 into `off`/`verify`), copy `redis_history` into `database_history` (`command_enc` into `body_enc`), and drop the two old tables. Settings or open-tab state that stores a Redis database id keeps working because ids are kept.

- Add `tests/fixtures/db` for 0007, and a migration test that opens the 0006 fixture, migrates and checks every Redis row, secret reference and history row survived.
- The Redis code switches to `databases` in the same phase (D1) and every existing Redis test must pass unchanged in behaviour.
- History caps: 500 rows per database and kind. Never store statements that carry a password (`create role … password`, `alter user … identified by`, `set password`, Mongo `createUser`/`updateUser`/`auth`), as with Redis.

## 3. Rust

### Layout

- `src-tauri/src/dbs/`: shared pieces moved out of `redis/`: `transport.rs` (direct, through a host, TLS; still no local listener), `store.rs` (the `databases` table, per-engine option validation), `gate.rs` (the shared types: `Mode`, `Class`, `Decision`, `Pending`, prepare/commit with the 2-minute binding to the connection, the typed-name check, master password to loosen production), `history.rs`, `saved.rs`, `transfer.rs` (export writers, import readers, dump/restore over SSH).
- `src-tauri/src/redis/`: unchanged features, now using `dbs::*`.
- `src-tauri/src/sql/`: `mod.rs` (an `SqlDriver` trait), `pg.rs`, `mysql.rs` (MySQL and MariaDB, with a flavour flag), `classify.rs`, `catalog.rs` (dialect queries for objects, columns, indexes, keys, activity, explain), `grid.rs` (turning grid edits into statements), `values.rs`.
- `src-tauri/src/mongo/`: `conn.rs`, `docs.rs`, `aggregate.rs`, `schema.rs`, `shell.rs` (parser), `classify.rs`.
- `commands/databases.rs`, `commands/sql.rs`, `commands/mongo.rs`: thin.

### Crates

Proposed, confirmed by the D0 spike (record versions and results in the D0 note):

- `tokio-postgres` (+ `postgres-types`, `rust_decimal`, `chrono` or `time` features as the tree already uses): run over our stream with `Config::connect_raw`. Expected to work; this is the reason Postgres goes first.
- `mysql_common` for MySQL and MariaDB (own protocol client; `mysql_async` cannot take our stream, D0 finding).
- `bson` for MongoDB (own `OP_MSG` client; the `mongodb` crate cannot take our stream, D0 finding).
- `sqlparser` for classifying SQL statements (Postgres, MySQL dialects).

**The spike question:** can `mysql_async` and `mongodb` run over a stream Dalan hands them (an SSH `direct-tcpip` channel)? The Redis rule stands: no local listener, because any program on the computer could use it. If a driver can't take our stream:

1. look for a supported hook (a custom connector, a stream factory, a test transport);
2. otherwise MySQL's wire protocol is small enough to drive directly with a focused crate that takes any stream (check `mysql_common` plus our own I/O), or write the handshake ourselves;
3. for MongoDB the wire protocol (`OP_MSG` + BSON) is also small, but replica set discovery, SRV and auth (SCRAM) would be ours to maintain;
4. stop and ask Mehrdad before choosing a local listener. If he accepts one as a fallback, it must bind `127.0.0.1` on a random port, accept exactly one connection per open, check the peer is our own process where the OS allows it, close immediately after, and never be used for production databases.

MongoDB replica sets and sharded clusters: with a stream-capable driver, each member is reached through the same route (a channel per member, opened on demand). Members are used by the names the replica set reports, resolved on the last hop.

### Values

- SQL: send rows to the UI as typed cells: `null`, bool, int (as string when it doesn't fit in a JS number), decimal (string, never float), float, text, bytes (hex preview, length), json, date/time (UTC epoch ms plus the original text for types with no zone), array, enum, uuid, interval, geometry (text form), unknown (text form). Never lose precision. Large values (text over 64 KB, bytes over 16 KB) arrive cut with "Load all".
- Mongo: extended JSON (canonical) both ways, so ObjectId, Decimal128, Long, Date, Binary, Timestamp, RegExp keep their types. The editor shows the relaxed shell form and converts back exactly.
- Paging: SQL keyset paging when the table has a primary key (stable while rows change), `offset` otherwise; counts use the planner estimate first ("estimate") with "Count exactly". Mongo uses `limit`/`skip` with `countDocuments` when cheap (indexed filter) and `estimatedDocumentCount` otherwise, labelled.
- Results stream to the UI over a `Channel` in batches (about 500 rows or 64 KB), with backpressure, and stop at the limit. Cancel works mid-stream (`pg_cancel_backend` on a second connection for Postgres, `KILL QUERY` for MySQL, `killOp` for Mongo, via the gate's "own session" exception).

### The gate for SQL (`sql/classify.rs`)

Parse with `sqlparser` in the database's dialect; a script is split into statements; each is classified:

| class | statements | Read-only | Confirm | Open |
| --- | --- | --- | --- | --- |
| read | `select` (without `for update`/`into`), `show`, `explain` (without `analyze`), `with … select`, `values`, `table` | run | run | run |
| write | `insert`, `update`/`delete` with a where clause, `merge`, `copy … from`, `call`, `do`, `select … for update`, `select … into`, `explain analyze` of a write, `lock`, `set` of session settings, `begin/commit/rollback/savepoint` (the manual-transaction verbs) | refuse | confirm | run |
| schema | `create`, `alter` (except type changes on existing columns), `comment`, `grant`, `revoke`, `refresh materialized view`, `vacuum`, `analyze`, `reindex` | refuse | confirm | confirm |
| dangerous | `drop`, `truncate`, `update`/`delete` without where (or `where true`/`1=1`), `alter … type`, `vacuum full`, `cluster`, `alter system`, `create`/`alter role … password`, `pg_terminate_backend`/`pg_cancel_backend`/`kill` of other sessions, anything that fails to parse | refuse | typed | typed |

Rules:

- **Second lock for read-only:** a database in Read-only runs every statement inside `start transaction read only` (Postgres) / `start transaction read only` (MySQL 5.6.5+), rolled back after. So a function that writes is stopped by the server. Postgres also sets `default_transaction_read_only = on` for the session.
- **No-where count:** for `update`/`delete` without where, count the target rows (`select count(*)`) and show the number in the dialog. For `update`/`delete` with where in Confirm mode, also run the statement inside a transaction with `returning`/row count, roll back, and show "This will change N rows", unless the table is huge (estimate over 1M), where it says so instead.
- **Grid edits** (`sql/grid.rs`): each changed row becomes `update … set changed columns where pk = … and changed columns = old values` (optimistic check); 0 rows updated means someone else changed it: stop, roll back, show the row. Tables without a primary key or unique not-null key are read-only in the grid ("This table has no key, so rows can't be matched safely. Use a query instead.").
- **Transactions:** the editor has Auto-commit (default) and Manual. In Manual, Dalan sends `begin` before the first statement and shows the bar (S37): statements not committed, age, locks held (`pg_locks` for own pid / `performance_schema.data_locks`). Idle over 2 minutes nudges; lock, quit, tab close, idle disconnect and app update all ask (S42). One connection per editor tab so transactions never mix; table data views share a pooled connection.
- **Statement timeout** per editor (default 30 s, setting) via `statement_timeout` / `max_execution_time`.
- Schema changes from S38 are generated in Rust from a typed diff (add/drop/rename column, type, null, default, index, foreign key), shown, and run in one transaction where the engine allows DDL in transactions (Postgres yes; MySQL no: say so and run statement by statement).

### The gate for MongoDB (`mongo/classify.rs`)

| class | operations | Read-only | Confirm | Open |
| --- | --- | --- | --- | --- |
| read | find, findOne, aggregate without `$out`/`$merge`, count*, distinct, explain, listCollections, listIndexes, stats | run | run | run |
| write | insert*, update*/replace*/delete* with a non-empty filter, findOneAnd*, bulkWrite, aggregate with `$out`/`$merge` | refuse | confirm (shows the match count and a sample) | run |
| schema | createIndex(es), createCollection, collMod (incl. validation), renameCollection, createView | refuse | confirm | confirm |
| dangerous | drop, dropDatabase, dropIndex(es), updateMany/deleteMany with an empty filter, killOp of others, shutdown, user and role changes | refuse | typed | typed |

- Read-only also sets read preference `secondaryPreferred` when the deployment has secondaries.
- Write confirms run the filter with `countDocuments` and show up to 3 matching documents ("Show sample").

### MongoDB shell (`mongo/shell.rs`)

No JavaScript engine. The shell accepts the familiar subset, parsed in Rust:

- `db.<collection>.<method>(<args>)` with chained cursor methods (`.sort()`, `.limit()`, `.skip()`, `.project()`, `.hint()`, `.collation()`, `.explain()`, `.count()`, `.toArray()`), `db.getCollection("name")`, `db.<method>()` for database commands (`runCommand`, `stats`, `getCollectionNames`, `currentOp`, `killOp`), `show dbs|collections|users`, `use <db>`, `it`.
- Arguments are relaxed JSON (unquoted keys, single quotes, trailing commas, comments) with shell helpers: `ObjectId()`, `ISODate()`, `new Date()`, `NumberDecimal()`, `NumberLong()`, `NumberInt()`, `UUID()`, `Timestamp()`, `BinData()`, `/regex/flags`.
- Anything else (variables, loops, functions) gets: "The shell runs single commands, not JavaScript. Use the Aggregations view or a saved pipeline for multi-step work." Change the board's subtitle copy to "the familiar db.collection syntax".

### Commands (thin; follow the Redis naming)

- Shared: `db_list`, `db_save`, `db_delete`, `db_test`, `db_parse_url` (fills the form from a connection string; the password goes straight to the vault, never back to the window), `db_set_mode`, `db_open(db_id, events)`, `db_close`, `db_history`, `db_history_clear`, `saved_list`, `saved_save`, `saved_delete`, `db_write_commit(pending_id, typed_name?)`, `db_cancel(conn_id, op_id)`.
- SQL: `sql_objects(conn, path)` (lazy tree), `sql_columns`, `sql_table_page(conn, table, filter, sort, columns, cursor, page_size, rows: Channel)`, `sql_cell(conn, table, pk, column)`, `sql_linked(conn, table, pk)`, `sql_run(conn, editor_id, text, mode: current|selection|all, limit, rows: Channel)` → result sets or `Pending`, `sql_explain(conn, text, analyze)`, `sql_complete(conn, text, cursor)` (catalog-based completion; cache the catalog per connection, refresh on demand and after DDL), `sql_format(text, dialect)`, `sql_txn(conn, editor_id, begin|commit|rollback|state)`, `sql_grid_prepare(conn, table, edits)` → `Pending`, `sql_structure(conn, table)`, `sql_structure_prepare(conn, table, diff)` → `Pending`, `sql_diagram(conn, tables, steps)`, `sql_activity(conn, view)`, `sql_session_action(conn, pid, cancel|end)` → `Pending`.
- Mongo: `mg_tree`, `mg_find(conn, coll, query, docs: Channel)`, `mg_explain`, `mg_doc_prepare(conn, coll, op)` → `Pending`, `mg_aggregate(conn, coll, pipeline, stage_upto, sample)`, `mg_pipeline_code(pipeline, language)`, `mg_schema(conn, coll, sample)`, `mg_indexes`, `mg_index_prepare` → `Pending`, `mg_validation`, `mg_validation_check(conn, coll, rules)` (count of failing documents), `mg_validation_prepare` → `Pending`, `mg_shell(conn, line)` → reply or `Pending`.
- Transfer: `export_start(source, format, options, path?)` (streams to a file chosen with the native save dialog in Rust; clipboard copies go through Rust too), `import_preview(file, target)`, `import_prepare(mapping)` → `Pending`, `backup_start(db, options, path)`, `restore_prepare(db, file)` → `Pending` (typed), `transfer_stop`.
- Every command returns typed results via tauri-specta (hard rule 6) and `AppError` (hard rule 7). New codes in the Redis style, prefixed `DB_` (shared), `SQL_`, `MONGO_`: for example `DB_AUTH_FAILED`, `DB_UNREACHABLE`, `DB_TLS_UNTRUSTED`, `DB_READ_ONLY_MODE`, `DB_CONFIRM_NEEDED`, `DB_TIMEOUT`, `DB_CANCELLED`, `SQL_PARSE`, `SQL_CONFLICT` (optimistic check failed), `SQL_NO_KEY`, `SQL_IN_TRANSACTION`, `MONGO_SHELL_UNSUPPORTED`, `DB_TOOL_MISSING` (dump tool not on the host). The server's own error text goes in `message`, as written (the Redis rule).

### Backup and restore

- Runs the engine's dump tool on the route's last hop over an SSH exec channel (`pg_dump -Fc`, `mysqldump --single-transaction`, `mongodump --archive --gzip`), streaming stdout to the chosen file on this computer with progress; stderr to the log. The password is passed through the tool's environment variable or a temporary file with mode 600 that is removed after (`PGPASSWORD`/`.pgpass`, `MYSQL_PWD` is deprecated: use `--defaults-extra-file`, `mongodump --config`), never on the command line.
- Direct routes run the tool on this computer if it is installed; otherwise the button explains what is missing.
- Restore is always typed confirmation, shows the target name and environment twice, and refuses production unless the database has been switched to Open with the master password.

## 4. UI

- Components under `app/components/databases/` (shared: list, sheet, engine picker, connection steps, write dialogs moved from `redis/` where they are generic), `app/components/sql/`, `app/components/mongo/`.
- Stores: `databases` (list), `useSqlStore(tabKey)`, `useMongoStore(tabKey)`, `savedQueries`.
- **Code editor:** CodeMirror 6 (`@codemirror/*`, with `@codemirror/lang-sql` and `@codemirror/lang-json`). This is a new UI dependency: it is the editor for SQL, pipelines, filters, validation rules and the document editor. Theme it from tokens (the syntax colours are the ANSI tokens). Completion comes from `sql_complete` / Mongo field names from the schema sample, never from the network.
- **Data grids:** one virtualised grid component for SQL tables, SQL results and Mongo's table view: column resize and reorder, sticky header, keyboard (arrows, Enter to edit, Esc, Tab, ⌘C copies TSV, ⇧-click range), cell states (changed, added, deleted, null, cut), type-aware editors (text, number, boolean, date-time with zone, enum select, JSON). 100,000 rows scroll smoothly.
- **Results:** multiple result sets as inner tabs; messages tab shows notices and row counts; copy as CSV/TSV/JSON/Markdown/SQL insert.
- **Charts** on results ("Chart" button): bar, line and scatter from a result's columns, using the dataviz rules in the design system. Small; can move to a later phase if time runs short.
- Add every new component to `replica/design/components.md` before building it, with states (loading, empty, error, read-only, disabled), keyboard and a11y, like the others.
- Settings (S23), "Databases" group: default write mode, default row limit (1000), statement timeout (30 s), confirm threshold for rows changed (always ask over 1,000 even in Open), editor font size, keep history.
- Empty states (our copy): no tables ("This schema has no tables yet."), no documents ("No documents match. Clear the filter?"), no saved queries ("Save a query with ⌘S and it shows up here for every database of that engine.").

## 5. Tests

- **Servers** (`tests/servers/compose.yml`): `postgres:16` (primary + one streaming replica), `mysql:8.4`, `mariadb:11`, `mongo:7` three-member replica set, each reachable only from the sshd container's network (the through-a-host path), plus one published port each for direct. Seed scripts: an `app` schema like the boards (orders, customers, order_items, payments, products, refunds with foreign keys, enums, jsonb, a view, a function that writes, 200k rows in one table), the same shape in MySQL/MariaDB, and the `shop` database for Mongo (mixed types on purpose: `placedAt` as a string in 3% of documents, Decimal128 and double totals).
- **Rust unit:** SQL classification table per dialect (every row of section 3, case and comment variants, multi-statement scripts, `with` + write, `explain analyze` of a write, unparseable input), grid edit SQL generation and the optimistic check, keyset paging cursors, value conversion (precision of numeric/bigint/decimal128, time zones), Mongo shell parser (every helper, chains, rejected JavaScript), Mongo classification, connection string parsing (and that the password never comes back), migration 0007 from the 0006 fixture.
- **Rust integration** (ignored unless servers are up): connect direct, through a host, through a jump chain, TLS verify and untrusted, wrong password; browse, page, edit and commit with a concurrent change (conflict); transactions and the open-transaction guard; read-only stops a writing function; explain; activity, cancel and end session; import and export round trips; backup and restore against a scratch database; Mongo find/edit/aggregate/schema/indexes/validation; replica set failover during a find.
- **Fake backend:** SQL and Mongo fakes in the browser-test backend with scripted gate decisions.
- **Browser (e2e):** F20 to F30; Redis e2e still green after D1; read-only hides edit controls; typed confirm stays disabled until the name matches; Cancel is focused; manual transaction bar and its close guard; lock closes every database connection.
- **Visual:** baselines for the 13 new boards' states in dark, and S36/S37 in light. **Accessibility:** axe on each view; grid and tree keyboard paths. **Walk:** a SQL chapter and a Mongo chapter in `pnpm walk` and `walk:app`.
- **Performance:** a 200k-row table pages and scrolls without stalls; a 1M-row result streams to the limit and cancels within 1 s; the object tree of a schema with 2,000 tables opens lazily under 300 ms; memory stays inside the `test-plan.md` budget.
- **Hand checks:** section J in `replica/testing/hand-checks.md`: a real staging Postgres through the bastion, a real MySQL/MariaDB if the team has one, a real Mongo replica set, a backup from a real host, and trying to break the gate from the editor and the shell.
- **features.csv:** one row per feature, `original` = `no`, following the file's rules (`pnpm check:features`).

## 6. Phases

Each is one `go`. Read this spec, `replica/redis/spec.md` and the boards named before each. Every phase keeps all Redis tests green.

- **D0 Driver spike:** prove `tokio-postgres`, `mysql_async` and `mongodb` over a `dbs::transport` stream through the Docker sshd, with TLS. Write the findings, versions and the chosen path per engine into the D0 note. If MySQL or MongoDB needs a local listener, stop and ask (section 3). No UI.
- **D1 Shared foundation (S34, S35; F20):** migration 0007 and its fixture and test; move transport, store, gate types, history into `dbs/`; Redis on `databases`; the all-engine list, the engine picker, the per-engine form, connection strings; saved queries table and the vault entry (empty).
- **D2 Postgres browse (S36 read side; F21):** connect, lazy object tree, table data with filter, sort, columns, keyset paging, estimates, cell panel, linked rows; the shared grid component.
- **D3 SQL editor (S37, S39 side panel; F23):** CodeMirror, completion from the catalog, run current/selection/all, streaming results, several result sets, messages, cancel, limit and timeout, history, saved queries, format, copy as, charts if time allows.
- **D4 SQL gate, edits and transactions (S42, S36 edits, S37 bar; F22):** `sqlparser` classification, read-only transactions, no-where counts, grid edits with the optimistic check, manual transactions and every close guard. The classification unit table is part of this phase.
- **D5 Structure, explain, diagram (S38, S39, S40; F24, F25 explain):** structure views and the typed diff to `alter`, explain tree and hint, diagram with export.
- **D6 Activity and transfer (S41, S43; F25, F26):** sessions, locks, blocking chains, cancel and end; export, import, backup and restore over SSH.
- **D7 MySQL and MariaDB:** the driver per D0, dialect catalog, explain (`explain format=json` / `analyze`), activity (`performance_schema`, `information_schema.processlist`), DDL outside transactions; every SQL view working for both.
- **D8 MongoDB documents (S44; F27):** connection per D0 (replica set, SRV, TLS, SCRAM and X.509), tree, find with filter/project/sort/collation, list/table/JSON views, the document editor with exact types, insert, delete, the Mongo gate for documents.
- **D9 MongoDB aggregation, schema, indexes, validation (S45, S46; F28, F29).**
- **D10 MongoDB shell and transfer (S47, S43 for Mongo; F30):** the parser, the shell, update/delete many through the gate, mongodump/mongorestore.
- **D11 Hardening and hand checks:** performance, walk chapters, visual baselines, accessibility, Settings group, `features.csv`, README/onboarding part, hand-checks section J. Ends `[h]`.

## 7. Not in this version

SQLite files and SQL Server (backlog; the picker hides them), visual query builder, schema compare and data sync between databases, scheduled jobs, model-first design (drawing tables to create them), stored procedure debugger, Mongo GridFS file browser (shown in the tree, opens read-only list only), Mongo change streams, sharing saved queries between teammates before sync exists.
