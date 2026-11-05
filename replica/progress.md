# Dalan build progress

Claude Code reads this file at the start of every phase. Typing `go` means: do the first phase below that is not checked. Each phase is sized for one working session.

Status keys: `[ ]` not started · `[~]` in progress (see notes) · `[x]` done.

## Phases

- [x] **P0 Scaffold (M0):**
  - git init.
  - Nuxt 4 SPA (`ssr: false`, srcDir `app/`) + Tauri 2, package `dalan-app`, pnpm, Node 24.12.0, dev port 4000.
  - Tailwind v4 + shadcn-vue + `@nuxtjs/color-mode` (dark default), with `replica/design/tokens.css` wired in.
  - Inter and JetBrains Mono bundled locally, plus `lucide-vue-next`.
  - Rust module skeleton (`db`, `vault`, `ssh`, `sftp`, `tunnels`, `keys`, `pty`, `import`, `logs`, `commands`, `error.rs`).
  - `rusqlite` + `rusqlite_migration` running `replica/schema.sql` as `0001_init`.
  - `AppError` → `{ code, message }`, and TS bindings generated from Rust into `app/types/bindings.ts`.
  - Lint, typecheck, Vitest, clippy.
  - A CI workflow running all checks plus the sweep and contrast scripts.
- [ ] **P1 Design primitives:**
  - `app/pages/design.vue` (dev only): every primitive and app component from `replica/design/components.md` in every state, dark and light side by side.
  - Use shadcn-vue components restyled by tokens; switch hover `accent` classes to `bg-surface-hover`.
  - Playwright screenshot of `/design`.
- [ ] **P2 Vault + unlock (S24):**
  - Argon2id + XChaCha20 vault, `secrets` table, `vault_*` commands, auto-lock.
  - First-run and unlock screens, as in the canvas.
- [ ] **P3 Hosts basic (S02, S03 minimal):**
  - `hosts_tree`, `host_create/update/delete/get` (address, port, username, password).
  - Hosts grid and list, empty state, simple editor sheet.
- [ ] **P4 Terminal slice (S18, F01, F11, F12):**
  - russh session manager.
  - `session_open/write/resize/respond/close/reconnect` over a Tauri Channel, with batched bytes and backpressure.
  - xterm.js view kept mounted.
  - Host-key trust-on-first-use, "identity changed" block, password prompt, disconnect and reconnect.
  - `known_hosts` table + S14 list.
  - **M1 done:** passes against the Docker test server.
- [ ] **P5 Groups, inheritance, tags (S02–S04, F04):**
  - `connection_profiles` resolution with "from group X" hints.
  - Nested groups, tags filter, drag to move.
- [ ] **P6 Keychain (S05–S08, F02):**
  - Generate and import keys (ssh-key), passphrases, identities, export to host, key auth in sessions.
- [ ] **P7 Navigation (S01, S16, S17, S23 basics, F03):**
  - Tab bar with status dots, New Tab, ⌘K palette, multiple sessions.
  - Settings: font, theme, scrollback, shortcuts list.
- [ ] **P8 Jump hosts and snippets (F05, S11, S12, S20):**
  - Host chains with a prompt per hop, startup snippet, snippets CRUD and packages, side panel run.
- [ ] **P9 Port forwarding (S09, S10, F06):**
  - Local, remote and SOCKS rules, start/stop, supervisor with backoff, autostart, port-in-use errors.
- [ ] **P10 SFTP (S21, S22, F07 minus edit):**
  - Dual pane: list, mkdir, rename, delete, chmod, hidden files, typed path.
  - Transfer queue with progress, cancel and conflicts.
- [ ] **P11 Workspaces (S19, S13, F08, F09):**
  - Split/focus, up to 16 panes, broadcast, templates, multi-host snippet run.
- [ ] **P12 Should-haves:**
  - SFTP edit-and-upload; env vars, proxy and agent forwarding.
  - Import from ssh_config and CSV (S25).
  - Themes, command history, local terminal, session logs (S15).
- [ ] **P13 Release pipeline:**
  - `tauri-action` builds, updater keys and `latest.json`, macOS signing and notarization hooks.
  - `site/` on GitHub Pages.
  - "Copy diagnostics".
- [ ] **P14 Could-haves:** in the order listed in `replica/architecture.md` → Build order → M4.

## Notes

(Claude Code appends one dated line per finished phase: what was built, what Mehrdad should test by hand, known gaps.)

- 2026-11-05 P0: Nuxt 4 SPA + Tauri 2 + Rust skeleton, `0001_init` migration (21 tables) runs on start, `AppError`, bindings via tauri-specta into `app/types/bindings.ts`, CI workflow. Test: `pnpm tauri dev` opens a window with the Dalan title; `vault.db` appears in `~/Library/Application Support/app.dalan.desktop/`. Gaps: placeholder app icon (flat dark square), TypeScript pinned to 5.9 (TS 7 breaks typescript-eslint/vue-tsc), `lucide-vue-next` is deprecated upstream in favour of `@lucide/vue` (kept per CLAUDE.md), window-state and single-instance plugins not added yet, shadcn-vue components added from P1.
