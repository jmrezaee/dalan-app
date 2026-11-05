# Dalan build progress

Claude Code reads this file at the start of every phase. Typing `go` means: do the first phase below that is not checked. Each phase is sized for one working session.

Status keys: `[ ]` not started · `[~]` in progress (see notes) · `[x]` done.

## Phases

- [x] **P0 Scaffold (M0):**
  - git init.
  - Nuxt 4 SPA (`ssr: false`, srcDir `app/`) + Tauri 2, package `dalan-app`, pnpm, Node 24.12.0, dev port 4000.
  - Tailwind v4 + shadcn-vue + `@nuxtjs/color-mode` (dark default), with `replica/design/tokens.css` wired in.
  - Inter and JetBrains Mono bundled locally, plus Lucide icons (`@lucide/vue`).
  - Rust module skeleton (`db`, `vault`, `ssh`, `sftp`, `tunnels`, `keys`, `pty`, `import`, `logs`, `commands`, `error.rs`).
  - `rusqlite` + `rusqlite_migration` running `replica/schema.sql` as `0001_init`.
  - `AppError` → `{ code, message }`, and TS bindings generated from Rust into `app/types/bindings.ts`.
  - Lint, typecheck, Vitest, clippy.
  - A CI workflow running all checks plus the sweep and contrast scripts.
- [x] **P1 Design primitives:**
  - `app/pages/design.vue` (dev only): every primitive and app component from `replica/design/components.md` in every state, dark and light side by side.
  - Use shadcn-vue components restyled by tokens; switch hover `accent` classes to `bg-surface-hover`.
  - Playwright screenshot of `/design`.
- [x] **P2 Vault + unlock (S24):**
  - Argon2id + XChaCha20 vault, `secrets` table, `vault_*` commands, auto-lock.
  - First-run and unlock screens, as in the canvas.
- [x] **P3 Hosts basic (S02, S03 minimal):**
  - `hosts_tree`, `host_create/update/delete/get` (address, port, username, password).
  - Hosts grid and list, empty state, simple editor sheet.
- [x] **P4 Terminal slice (S18, F01, F11, F12):**
  - russh session manager.
  - `session_open/write/resize/respond/close/reconnect` over a Tauri Channel, with batched bytes and backpressure.
  - xterm.js view kept mounted.
  - Host-key trust-on-first-use, "identity changed" block, password prompt, disconnect and reconnect.
  - `known_hosts` table + S14 list.
  - **M1 done:** passes against the Docker test server.
- [x] **P5 Groups, inheritance, tags (S02–S04, F04):**
  - `connection_profiles` resolution with "from group X" hints.
  - Nested groups, tags filter, drag to move.
- [x] **P6 Keychain (S05–S08, F02):**
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
- 2026-11-05 P1: shadcn-vue primitives restyled by tokens (hover `accent` -> `bg-surface-hover`), app components (TabBar/AppTab, VaultNav, PageToolbar, HostCard, GroupCard, KeyCard, RuleCard, StatusOverlay, UnlockCard, ProgressRow, PaneHeader, Stepper, SplitButton, IconButton, PasswordInput, NumberInput, SearchInput, EmptyState, StatusDot/Pill, TagChip), and `/design` (dev only) with dark and light side by side. Test: `pnpm dev`, open http://127.0.0.1:4000/design; `pnpm screenshot:design` writes `replica/design/design-page.png` (uses installed Chrome). Gaps: FileTable/PathBar, Workspace split (Resizable), SidePanel, NewTab, ShortcutRecorder, CredentialPicker, TagInput, ChainPicker, EnvVarList, HostPicker, ConflictDialog are built with their screens (P3 to P11); Reka dialogs/menus/toasts follow the app theme, not the light island; `.light` class added to tokens.css (via build_css.py) so a light island can sit inside the dark root; switched icons to `@lucide/vue` (successor of deprecated `lucide-vue-next`).
- 2026-11-05 P2: Argon2id (64 MiB, t=3) + XChaCha20-Poly1305 vault in Rust (`vault/kdf, crypto, secrets`), key held in `Zeroizing` memory and wiped on lock; commands `vault_status/create/unlock/lock/change_password/touch`; 5 wrong tries start a 30 s cooldown; idle auto-lock (setting `security.auto_lock_minutes`, default 15) with a 30 s warning toast; `VaultGate` + S24 screens (strength meter, caps-lock hint, shake, countdown). Test: `pnpm tauri dev`, create a vault (8+ chars), press Lock, try wrong passwords, unlock; quit and reopen to see the unlock screen. Gaps: no real-window run of the UI flow (UI was checked in a browser against a stub of the Rust side, Rust by 13 unit tests); orphan-secret sweep on unlock and tunnel autostart wait for P3/P9; no `change_password` screen yet (command is ready, lives in Settings P7); auto-lock minutes not yet editable (Settings P7); lock does not yet close sessions (none exist).
- 2026-11-05 P3: `hosts` module + commands `hosts_tree`, `host_get`, `host_save` (create/update in one transaction, password goes into `secrets`, replaced in place or removed on clear), `host_delete` (removes profile and secret); every host command needs an unlocked vault via new `AppState::with_db_session`. UI: hosts store, `/` is now the Hosts page (S02: grid/list toggle remembered, search, empty state, ⌘N, delete confirm) and `HostEditorSheet` (S03 minimal, ⌘S, "Unsaved changes", saved-password state). Test: `pnpm tauri dev`, unlock, Add host (label, address, port, user, password), edit it (password shows dots + Change), switch to list view, search, delete; quit, reopen, unlock, the host is still there. Gaps: no Connect yet (P4), groups/tags/inheritance (P5), "Import" button disabled (P12), host_get returns only the host itself, not the resolved profile (P5), no sidebar sections besides Hosts and no tab bar (P7), no real-window run (UI checked in a browser against a stub of the Rust side; Rust by 4 new unit tests, 17 total).
- 2026-11-05 P4: russh session manager (`ssh/`): one task per tab, host-key check against `known_hosts` (unknown: ask and save only on yes; changed: blocking prompt, only an explicit typed-name replace continues), password auth (saved, or prompted with Remember), PTY shell, output batched every 8 ms / 64 KB and sent as raw bytes on a binary Tauri channel, backpressure by UI acks (pauses reading at 1 MB behind), keepalive 30 s x3, reconnect on the same tab, vault lock closes all sessions. Commands `session_open/write/resize/ack/respond/close/reconnect`, `known_hosts_list/delete`; `host_delete` refuses while the host has a live tab. UI: tab bar (Vault + one tab per session, v-show so xterm never unmounts), `TerminalView` (xterm + fit + WebGL), `SessionPane` with the S18 overlays (connecting, first-connect, identity changed, password, sign-in failed, can't reach, connection closed), Known hosts page (S14). Tests: 8 new Rust tests incl. an in-process SSH server (first connect, known key, changed key, cancel, wrong password, Remember, unreachable) and an ignored test against the Docker server (`cargo test docker -- --ignored`). Test by hand: start the Docker server from CLAUDE.md (port 2222, user test, password test), `pnpm tauri dev`, add host 127.0.0.1:2222 test/test, double-click it, trust the key, use the shell (try `seq 1 100000`), `docker restart dalan-test-ssh` to see the closed overlay and Reconnect, delete the host's row in Known hosts then reconnect to see the prompt again; for F12 edit `known_hosts.public_key` of that row in vault.db then connect to see the block. Gaps: not run in the real window (UI checked in a browser against a stub, Rust against real OpenSSH in a test); only password auth (keys P6), no keyboard-interactive, host certificates refused, no jump hosts (P8), no startup snippet, OS detection or logs, ⌘T/New Tab screen, shortcuts and copy/paste settings are P7, terminal font/theme fixed, ssh-rsa SHA-1 not offered, session_write sends keystrokes as a JSON number array (fine for typing, slow for huge pastes).
- 2026-11-05 P5: groups (nested, own profile each), per-field inheritance host -> group -> parent group -> default for username, port and password, with the source group reported (`inherited`, `effective_*`) so editors show "from Production"; connecting uses the resolved values, including an inherited password; tags (created on save, removed when unused, filter in the sidebar, all-selected must match). Commands `group_save`, `group_delete` (children move up, password removed, one transaction), `hosts_move`; `host_save` takes `group_id` and `tags`; `known`-style cycle check refuses putting a group inside itself. UI: `HostsView` (breadcrumb, group cards, host grid/list, New host ▾ New group), `GroupEditorSheet` (S04), host editor gets group select, tag input and inherited hints, drag a host onto a group card or breadcrumb to move it. Test: `pnpm tauri dev`: New host ▾ New group "Production" with username and password; New group "Web" inside it with port 2222; add a host in Web with no username/password and connect (Docker server: port 2222, test/test) - it should sign in with the group's values; edit the host to see "from Production"; drag a host onto a group card; delete the middle group and check its children moved up. Gaps: only username, port and password inherit (key/identity P6, proxy/jump chain P8, env/theme/font later); no multi-select drag, no dragging groups (use Inside in the group editor); no tags management page; real-window drag and drop not run (UI checked in a browser against a stub, Rust by 5 new tests, 30 total).
- 2026-11-05 P6: keychain in Rust (`keys`): generate Ed25519, ECDSA P-256/384/521, RSA 4096 (built outside the DB lock), import by paste or by a file chosen in the dialog (Rust reads it; `needs_passphrase` / `invalid_key` / wrong-passphrase errors), private key stored in `secrets` as OpenSSH text (encrypted with the user's passphrase when given), saved passphrase optional and checked before saving, rename, delete (removes secrets, hosts fall back to inherited sign-in), identities (username + password or key). Inheritance now also resolves key and identity per level (`groups::resolve`, a level can set a field directly or through its identity). Sessions: key first, then saved password; encrypted key asks for its passphrase (Remember option); a refused key with no password shows `{host} rejected the key "{label}".`. `key_export_to_host` (S08) reuses the session machinery: connects, asks the usual host-key/password questions inside the dialog, appends the public key to `authorized_keys` (folder 700, file 600, no duplicate, path charset checked), then links the key to the host. UI: Keychain section (S05) with key cards (copy public key, Install on a server, Edit, Delete) and identities, Generate / Import / Edit / Install dialogs and the identity sheet, Identity and Key selects in the host and group editors with "from group" hints, passphrase prompt overlay. Test: Docker server up (port 2222, test/test), `pnpm tauri dev`, Keychain -> Generate key (Ed25519), key menu -> Install on a server -> pick your Docker host (password test, trust the key) -> then edit the host, remove its password, connect: it should sign in with the key alone; also try a passphrase on the key (asked on connect unless saved) and Import with a pasted key. Gaps: not run in the real window (UI checked in a browser against a stub; Rust against real OpenSSH in `cargo test docker -- --ignored`, incl. the full F02), no install-then-test-sign-in step, no drag-and-drop of key files, no key certificates (S05) or FIDO2, `ssh-rsa` SHA-1 not offered, no agent, a key's passphrase prompt is per connect (not per session reuse), identities are chosen per host/group but not shown as their own tab in the credential picker (two plain selects instead of the combined CredentialPicker), 13 new Rust tests (43 total + 2 ignored).
