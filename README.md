# Dalan

A private, local-first SSH client for you and your team. Built with Nuxt 4, Tauri 2 and Rust.

## Databases (Redis, PostgreSQL, MySQL, MariaDB, MongoDB)

Dalan opens databases: saved under Vault > Databases, reached directly or **through a saved SSH host** (Dalan signs in to the host and talks to the database from there; nothing listens on your computer). The same write modes apply to every engine.

- **SQL (PostgreSQL, MySQL, MariaDB):** tables with paging and filters, a query editor with a result per statement, reviewed grid edits, structure changes, plans, a relationship diagram, activity (sessions, cancel, end), CSV/JSON/SQL export, CSV import and backups with the engine's own tools. Every statement is classified in Rust: reads run, changes ask (with the number of rows they reach), dangerous ones ask for the database's name. The spec is `replica/databases/spec.md`.
- **MongoDB:** collections with filter, projection, sort and collation; list, table and JSON views; edits with exact types; an aggregation builder; schema, indexes and validation; a shell in the familiar `db.collection` syntax (no JavaScript); export and `mongodump` backups.
- **Limits:** Settings > Databases sets how many rows a SQL query shows and how long it may run.

### Redis

Redis: single servers, Cluster and Sentinel.

- **Browser:** keys listed with `SCAN` (never `KEYS`), grouped into folders; values of every type in pages.
- **Write modes, per database:** Read-only (writes are refused before they are sent), Confirm writes (every change shows the exact command first), Open. Dangerous commands and bulk actions always ask for the database's name. The check is in Rust; a production database needs the master password to become less protected.
- **Console, Pub/Sub, Server:** commands with completion and encrypted history; listening on its own connection; the server's numbers and slow commands.

The spec is `replica/redis/spec.md`; the test servers are in `tests/servers/compose.yml` (`scripts/test-servers.sh up`).

## Develop

```bash
nvm use 24.12.0
pnpm install
pnpm tauri dev       # app with hot reload (UI on :4000)
pnpm lint && pnpm typecheck && pnpm test
cd src-tauri && cargo clippy -- -D warnings && cargo test
```

`app/types/bindings.ts` is generated from Rust. Run `cargo test` (or start the app) to refresh it.
