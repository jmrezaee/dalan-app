# Dalan

A private, local-first SSH client for you and your team. Built with Nuxt 4, Tauri 2 and Rust.

## Develop

```bash
nvm use 24.12.0
pnpm install
pnpm tauri dev       # app with hot reload (UI on :4000)
pnpm lint && pnpm typecheck && pnpm test
cd src-tauri && cargo clippy -- -D warnings && cargo test
```

`app/types/bindings.ts` is generated from Rust. Run `cargo test` (or start the app) to refresh it.
