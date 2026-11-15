# Dalan

A private, local-first SSH client for you and your team. Built with Nuxt 4, Tauri 2 and Rust.

## Databases (Redis)

Dalan also opens Redis: saved under Vault > Databases, reached directly or **through a saved SSH host** (Dalan signs in to the host and talks to Redis from there; nothing listens on your computer). Single servers, Cluster and Sentinel.

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
