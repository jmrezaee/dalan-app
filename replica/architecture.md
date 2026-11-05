# Architecture: termius-app (a rebuild of Termius's core features)

Built from `replica/recon.md` (screens S01–S25, flows F01–F12) and `replica/features.csv`.
Date: 2026-11-04. Scope: desktop (macOS first, then Windows and Linux), one user, one local encrypted vault. There are no accounts, sync or billing.

## Shape of the app

```
┌──────────────────────── Tauri window ────────────────────────┐
│  Webview: Nuxt 4 SPA (ssr:false) + shadcn-vue + Tailwind     │
│  Pinia stores · xterm.js · composables wrapping invoke()     │
│        │ invoke(command)            ▲ Channel<events>        │
├────────┼────────────────────────────┼────────────────────────┤
│  Rust core (src-tauri)                                       │
│  vault (Argon2id + XChaCha20) · db (rusqlite)                │
│  ssh (russh) · sftp (russh-sftp) · tunnels · pty (local)     │
│  keys (ssh-key) · known_hosts · import · logs                │
└──────────────────────────────────────────────────────────────┘
          │ TCP/SSH                     │ files
      your servers               app data dir: vault.db, logs/, edit-tmp/
```

Rules that hold everywhere:

- **Secrets never cross into JavaScript.** The webview sends a password or private key to Rust once, when it is entered. After that it only sees `has_password: true`. Decrypted keys live in Rust memory, wrapped in `zeroize`, and are dropped on lock.
- **The webview never touches the network or the filesystem.** All SSH, SFTP, tunnel, file and database work runs in Rust commands, so the Tauri capability file stays minimal.
- **Bytes stay bytes.** Terminal output goes from Rust to xterm.js as raw `Uint8Array` chunks. It is never turned into a `String` in Rust, because a multi-byte UTF-8 character can be split across packets.

## Stack

| layer | choice | why |
| --- | --- | --- |
| UI | Nuxt 4, `ssr: false`, TypeScript, Pinia | Your existing stack. Tauri serves a static SPA. |
| components | shadcn-vue (Reka UI) + Tailwind v4 | Your stack. Command, Dialog, Sheet, ContextMenu, Resizable, Tabs cover most of the UI. |
| terminal | `@xterm/xterm` + addons fit, webgl, search, unicode11, web-links | The standard browser terminal (VS Code uses it). The webgl renderer keeps big outputs smooth. |
| desktop shell | Tauri 2 | Small binary and native performance. Rust is where SSH lives anyway. |
| SSH / SFTP | `russh` (client, agent, port forwarding) + `russh-sftp` | Pure Rust and async on tokio. Supports direct-tcpip for jump hosts and forwarded-tcpip for remote forwards. |
| key handling | `ssh-key` (+ `rand_core`) | Generates and parses OpenSSH keys (Ed25519, ECDSA, RSA), encrypted keys and certificates, and fingerprints. |
| local terminal | `portable-pty` | One API over Unix PTYs and Windows ConPTY. |
| database | SQLite through `rusqlite` (`bundled` feature) + `rusqlite_migration` | Embedded, a single file, no server, relational. One DB file in the app data dir. |
| encryption | `argon2` (Argon2id KDF) + `chacha20poly1305` (XChaCha20-Poly1305) + `zeroize` | Each secret is encrypted with a key derived from the master password. Optional later: SQLCipher for the whole file. |
| file watching | `notify` | For the SFTP edit-locally-then-upload flow (F07). |
| ids | `uuid` v7 | Time-ordered, so they sort by creation and are ready if you add sync later. |
| errors / serde | `thiserror`, `serde`, `serde_json` | Typed errors that turn into `{ code, message }` for the UI. |
| Tauri plugins | dialog, clipboard-manager, opener, window-state, single-instance, log; updater later | Only what the UI needs. No `fs` or `http` plugin for the webview. |
| auth | none: master password unlock (S24) | Single local user. |
| payments / email | none | Personal and team tool. |
| jobs | tokio tasks inside the app | See "Background tasks". |
| distribution | GitHub Releases via `tauri-action`; macOS Developer ID + notarization | Not the Mac App Store: its sandbox blocks the local terminal and broad SSH use. |
| tooling | pnpm, Node 24.12.0 (`nvm use 24.12.0`), Rust stable, `cargo clippy`, Vitest, Playwright later | |

Pin versions with the latest stable at scaffold time and commit both lockfiles.

## Project layout

```
termius-app/                 local folder (kept); package, crate and repo name: dalan-app
  app/                         Nuxt 4 srcDir
    pages/                     vault/hosts.vue, vault/keychain.vue, vault/port-forwarding.vue,
                               vault/snippets.vue, vault/known-hosts.vue, vault/logs.vue, sftp.vue, settings.vue
    components/
      shell/                   TabBar, CommandPalette, NewTab
      hosts/  keychain/  snippets/  forwards/  sftp/  terminal/  workspace/
      ui/                      shadcn-vue generated
    composables/               useTauri (typed invoke), useSession, useShortcuts
    stores/                    vault, hosts, keychain, sessions, tabs, sftp, transfers, settings
    types/bindings.ts          generated from Rust (tauri-specta or ts-rs). Never hand-written.
  src-tauri/
    src/
      main.rs  lib.rs          builder, state, plugin + command registration
      error.rs                 AppError -> { code, message }
      db/  (mod.rs, migrations/0001_init.sql = replica/schema.sql, repo per table)
      vault/                   kdf, crypto, Secret store, lock state
      keys/                    generate, import, export_to_host
      ssh/                     session manager, connect (chain, proxy), handler (host key), auth, agent
      sftp/                    browse ops, transfer queue, edit watcher
      tunnels/                 local, remote, dynamic (SOCKS5)
      pty/                     local terminal
      import/                  ssh_config, known_hosts, csv
      logs/                    recorder, encrypted chunk writer/reader
      commands/                thin #[tauri::command] fns per area, calling the modules above
    capabilities/default.json
```

**Terminal tabs are not routes.** Vault sections and Settings are pages. Sessions, workspaces and SFTP stay mounted in a keep-alive layer and are shown or hidden with `v-show`. An xterm instance must never unmount while its session is alive, or its scrollback is lost.

## Schema

Tables: **21**. See `replica/schema.sql`, which is migration `0001_init`. It was tested in SQLite: FK delete rules, the port-forward check and unique bind constraints, and inheritance resolution all behave as intended.

Access rules: **data-layer checks in Rust.** There is one user, so SQLite has no row-level security. Every command goes through a `require_unlocked()` guard, and every query is scoped to `vault_id`.

| group | tables |
| --- | --- |
| vault | `vaults` (KDF salt, params, key-check), `secrets` (all ciphertext) |
| keychain | `keys`, `identities` |
| hosts | `groups`, `hosts`, `connection_profiles`, `profile_env_vars`, `profile_jump_hosts`, `tags`, `host_tags` |
| snippets | `snippet_packages`, `snippets`, `snippet_targets` |
| security | `known_hosts` |
| tunnels | `port_forwards` |
| workspace & history | `workspace_templates`, `connection_history`, `command_history`, `session_logs` |
| app | `settings` |

**Inheritance design (recon F04).** Every host and every group owns one `connection_profiles` row. A NULL column means "inherit". Resolution walks host → group → parent group → … → app defaults, one field at a time:

```sql
WITH RECURSIVE chain(depth, profile_id, group_id) AS (
  SELECT 0, profile_id, group_id FROM hosts WHERE id = :host
  UNION ALL
  SELECT c.depth + 1, g.profile_id, g.parent_id FROM chain c JOIN groups g ON g.id = c.group_id
)
SELECT p.* , c.depth FROM chain c JOIN connection_profiles p ON p.id = c.profile_id ORDER BY c.depth;
```

Rust folds those rows into a `ResolvedProfile` and records which group each value came from. The host editor uses that to show "from group Prod" hints (S03). Env vars use `env_mode` (inherit, merge or replace). Jump chains use `chain_mode` (inherit, none or custom).

**Delete rules, decided:**

- A group cannot be deleted while it has children (`RESTRICT`). The command moves children to the parent, then deletes, in one transaction.
- A host used as a jump hop cannot be deleted (`RESTRICT`). The UI says which hosts use it.
- Deleting a host cascades to its tags, forwards, logs and history.
- Deleting a key or identity sets profile references to NULL (`SET NULL`). The host falls back to inherited credentials.
- Secrets are deleted by the owning command. An orphan sweep runs on unlock.

**Hard constraints in the DB:**

- Two local or dynamic forwards cannot bind the same `address:port` (partial unique index).
- A dynamic rule has no destination, and the other kinds must have one (CHECK).
- Known hosts are unique per `host:port` + key type.
- Key fingerprints are unique per vault, so the same key can't be imported twice.

## API: Tauri commands

Notation: `cmd_name(input) -> output`. Every command except `vault_status`, `vault_create` and `vault_unlock` requires an unlocked vault. Errors come back as `{ code, message }`. Codes include `locked`, `wrong_password`, `too_many_attempts`, `needs_passphrase`, `invalid_key`, `not_found`, `validation`, `in_use`, `auth_failed`, `host_unreachable`, `host_key_changed`, `kex_failed`, `port_in_use` and `permission_denied`.

Streams use **Tauri `Channel`s** passed as a command argument. They are ordered and per call, which fits a terminal better than global events.

### Vault (F10, S24)

| command | does | input | output | flow |
| --- | --- | --- | --- | --- |
| `vault_status` | first run? locked? | – | `{ exists, unlocked, auto_lock_minutes, cooldown_secs? }` | F10 |
| `vault_create` | derive key, write vault row and key-check | `{ master_password }` | `()` | F10 |
| `vault_unlock` | verify key-check, hold key in memory, autostart tunnels, sweep orphans | `{ master_password }` | `()` or `wrong_password` | all |
| `vault_lock` | drop key, close sessions (confirm in UI) | – | `()` | – |
| `vault_touch` | report user activity so auto-lock counts idle time | – | `()` | – |
| `vault_change_password` | re-derive and re-encrypt all secrets in one transaction | `{ old, new }` | `()` | – |

### Hosts, groups, tags (F01, F04, F05; S02–S04)

| command | does | input | output |
| --- | --- | --- | --- |
| `hosts_tree` | groups + hosts for the list (flat, client builds the tree) | `{ group_id? , query?, tag_ids? }` | `{ groups[], hosts[] }` with tag ids, `has_password` flags |
| `host_get` | one host with its own profile and its resolved profile | `{ id }` | `{ host, profile, resolved, sources }` |
| `host_save` (create when `id` is null, else update) | upsert host + profile + env + chain + tags; password goes into `secrets` | `HostInput { id?, label, address, port?, username?, password?, clear_password }` | `HostSummary` |
| `host_delete` | delete; `in_use` if it is a jump hop | `{ id }` | `()` |
| `hosts_move` | drag into a group (F04) | `{ ids[], group_id? }` | `()` |
| `group_create` / `group_update` | upsert group + profile | `GroupInput` | `GroupSummary` |
| `group_delete` | move children up, then delete | `{ id }` | `()` |
| `tags_list` / `tag_delete` | tags menu | – / `{ id }` | `Tag[]` / `()` |

### Keychain (F02; S05–S08)

| command | does | input | output |
| --- | --- | --- | --- |
| `keys_list` | keys with public part + fingerprint | – | `KeySummary[]` |
| `key_generate` | Ed25519 / ECDSA / RSA | `{ label, type, bits?, passphrase?, save_passphrase }` | `KeySummary` |
| `key_import` | paste text or file path (picked through the dialog plugin, read in Rust) | `{ label, pem? , path?, passphrase?, save_passphrase }` | `KeySummary` or `needs_passphrase`, `invalid_key` |
| `key_set_certificate` | attach an OpenSSH certificate | `{ id, certificate }` | `KeySummary` |
| `key_update` / `key_delete` | rename, change saved passphrase / delete | | |
| `key_export_to_host` | connect, append the public key to `authorized_keys` (create `~/.ssh` 700, file 600), link it to the host | `{ key_id, host_id, dir?, file_name? }` | `()`, streams prompts via `Channel<ConnectEvent>` |
| `identities_list` / `identity_create` / `identity_update` / `identity_delete` | | `IdentityInput` | `IdentitySummary` |

### Sessions: SSH and local terminal (F01, F03, F05, F11, F12; S16–S20)

| command | does | input | output |
| --- | --- | --- | --- |
| `session_open` | resolve profile, connect (proxy → chain → target), auth, open PTY shell, run startup snippet, start log | `{ target: {host_id} or "local", cols, rows, on_event: Channel<SessionEvent> }` | `{ session_id }` |
| `session_write` | keystrokes / paste / broadcast | `{ session_id, data: bytes }` | `()` |
| `session_resize` | window-change | `{ session_id, cols, rows }` | `()` |
| `session_respond` | answer a prompt raised on the channel | `{ session_id, prompt_id, answer }` | `()` |
| `session_close` | | `{ session_id }` | `()` |
| `session_reconnect` | new connection, same channel and tab (F11) | `{ session_id }` | `()` |
| `recent_connections` | New Tab list (S16) | `{ limit }` | `RecentItem[]` |
| `command_history_list` | side panel (S20), later autocomplete | `{ host_id?, query? }` | `string[]` |

`SessionEvent` (Rust → UI, on the channel):

```
State      { state: connecting | authenticating | ready | disconnected | closed, hop?: "bastion" }
Data       { bytes }                                  (batched, see "parts that bite")
Prompt     { prompt_id, kind: host_key_new    { host, key_type, fingerprint }
                             | host_key_changed{ host, old_fingerprint, new_fingerprint }   (blocking, default = cancel)
                             | password | passphrase { key_label } | keyboard_interactive { name, instructions, fields[] } }
Error      { code, message }                         (auth_failed, host_unreachable, kex_failed, ...)
Exit       { status? }
Hop        { via: ["bastion", ...] }                 (jump hosts in order, once, before the first hop connects)
OsDetected { os }
```

**Host-key flow (F01, F12).** russh's `check_server_key` handler compares the key with `known_hosts`. On a match it continues. If the host is unknown, it sends a `host_key_new` prompt and awaits a `oneshot` answer (trust once, trust and save, or cancel, with a timeout). If the key changed, it sends `host_key_changed`. The only way to proceed after a change is to delete the old entry in S14, or "Replace and connect", which needs a typed confirmation.

### Snippets (F08; S11–S13, S20)

| command | does |
| --- | --- |
| `snippets_list` | snippets + packages |
| `snippet_create` / `snippet_update` / `snippet_delete` | includes `default_target_host_ids` |
| `package_create` / `package_update` / `package_delete` | |
| `snippets_move` | into a package |

Running a snippet needs no extra command. The UI opens sessions (`session_open`), waits for `ready`, then calls `session_write(script + "\n")`. Multi-host run (F08) does that for each pane of a new workspace.

### Port forwarding (F06; S09–S10)

| command | does | input | output |
| --- | --- | --- | --- |
| `forwards_list` | rules + live status | – | `ForwardSummary[]` |
| `forward_create` / `forward_update` / `forward_delete` | | `ForwardInput` | |
| `forward_start` | connect (or reuse a live connection to that host), bind the listener; `port_in_use` on bind error | `{ id, on_event: Channel<TunnelEvent> }` | `()` |
| `forward_stop` | | `{ id }` | `()` |

`TunnelEvent`: `State { stopped | starting | active | reconnecting | error }`, `Connections { open }`, `Prompt` (same as sessions), `Error`.

### SFTP (F07; S21–S22)

One interface for both panes. An `endpoint` is `"local"` or an `sftp_id`.

| command | does | input | output |
| --- | --- | --- | --- |
| `sftp_open` / `sftp_close` | SFTP subsystem on a (shared) SSH connection | `{ host_id, on_event }` / `{ sftp_id }` | `{ sftp_id, home }` |
| `fs_list` | directory listing | `{ endpoint, path, show_hidden }` | `Entry[]` (name, kind, size, mtime, mode, symlink target) |
| `fs_mkdir` / `fs_rename` / `fs_remove` / `fs_chmod` / `fs_realpath` | | `{ endpoint, ... }` | |
| `transfer_start` | queue copy local↔remote or remote↔remote (streamed through the app), recursive | `{ items[{src_endpoint, src_path, dst_endpoint, dst_dir}], on_conflict: ask, on_event: Channel<TransferEvent> }` | `{ transfer_id }` |
| `transfer_resolve` | answer a conflict | `{ transfer_id, item, choice: replace or keep_both or skip, apply_to_all }` | `()` |
| `transfer_cancel` | | `{ transfer_id }` | `()` |
| `edit_open` | download to `edit-tmp/`, open with the default or chosen app, watch the file | `{ sftp_id, path, app? }` | `{ edit_id }` with `changed` events → UI asks "upload?" |
| `edit_upload` / `edit_close` | upload (check the remote mtime first, warn if it changed) / stop watching, delete the temp file | `{ edit_id }` | `()` |

`TransferEvent`: `Progress { item, bytes, total }`, `Conflict { item, existing }`, `ItemDone`, `ItemFailed { code }`, `Done`.

### Known hosts, workspaces, logs, import, settings

| command | does | flow / screen |
| --- | --- | --- |
| `known_hosts_list` / `known_host_delete` | S14 | F12 |
| `workspaces_list` / `workspace_save` / `workspace_delete` | templates (S19) | F09 |
| `logs_list` / `log_read` (chunked, `{ id, offset, limit }`) / `log_bookmark` (`{ id, bookmarked, comment }`) | S15 | – |
| `import_ssh_config_preview` | parse `~/.ssh/config` (+ Include), known_hosts and keys it references; returns rows + warnings (ProxyCommand ignored, wildcards) | F10, S25 |
| `import_csv_preview` | CSV in Termius's column layout | S25 |
| `import_commit` | write the chosen rows in one transaction | S25 |
| `settings_get` / `settings_set` | JSON per section | S23 |
| `app_info` | version, data dir, platform | S23 |

**Command count: 77** across 10 areas. The command palette (S17) is client-side fuzzy search over Pinia stores, so it needs no command.

Webhooks in or out: **none** (local app).

### Background tasks (tokio, in-process)

| task | when | does |
| --- | --- | --- |
| keepalive | per connection, every `keepalive_secs` (default 30) | Detects dead links. Marks the session `disconnected`. |
| tunnel supervisor | per active forward | Restarts with backoff (1s → 30s) when its SSH connection drops. Autostarts rules on unlock. |
| auto-lock | idle N minutes (setting) | `vault_lock`, after a UI warning. |
| log recorder | per session | Appends encrypted chunks. On session end, deletes the host's previous non-bookmarked log. |
| edit watcher | per open edit | `notify` events, debounced 500 ms → `changed` event. |
| orphan sweep | on unlock | Deletes `secrets` rows nothing references, and stale `edit-tmp/` files. |
| connection pool | always | One SSH connection per host, shared by shell, SFTP and tunnels when credentials match. It closes 60 s after the last user leaves. |

## The parts that bite

- **Terminal throughput and backpressure.** `cat` on a big file or `yes` can flood the IPC. Batch output in Rust (flush every ~8 ms or 64 KB) and send raw bytes. Respect xterm's `write(data, callback)`: when the UI is more than ~1 MB behind, pause reading from the SSH channel so the window stays responsive.
- **Async host-key prompts.** `check_server_key` must `await` a user answer while the connection is open. Use a `oneshot` with a timeout, and handle the tab closing mid-prompt.
- **Jump chains.** Each hop is a full SSH connection over the previous hop's `direct-tcpip` channel. Each hop has its own host-key check and auth prompts, and the UI shows which hop is asking.
- **Agent forwarding.** The server opens `auth-agent` channels back to you. Answer them from the vault's decrypted keys, and only for that session. Never forward to an untrusted host by default.
- **Key formats.** OpenSSH (encrypted with bcrypt-pbkdf), PEM PKCS#1/PKCS#8 and RSA SHA-2 signatures (`rsa-sha2-256/512`) are must-haves. Old servers may only offer `ssh-rsa` (SHA-1); expose it as an opt-in setting. PuTTY `.ppk` is a could.
- **FIDO2 (`sk-*`) keys.** russh support is limited, so this is the riskiest could item. Prototype it before promising it.
- **Secrets at rest.** Hostnames, labels and snippets sit in plaintext SQLite. Only secrets are encrypted. If that's not enough, switch rusqlite to the SQLCipher feature, keyed from the same master key. Decide before you have real data.
- **Sleep and network changes.** macOS sleep kills TCP silently. Keepalive plus a `disconnected` overlay with Enter-to-reconnect (F11) covers it. Never auto-retry a password.
- **Port binding.** Default to `127.0.0.1`. Report `port_in_use` clearly. Remote forwards fail silently unless the server allows them, so give the error a hint about `GatewayPorts`.
- **SFTP performance and edge cases.** Pipeline several read/write requests per file, because sequential 32 KB requests crawl on high-latency links. Handle symlinks, permission-denied folders, huge directories (stream or virtualise the list), cancelling mid-file (delete the partial file) and name conflicts.
- **Races.**
  - A double-click on connect opens two tabs: debounce per host.
  - Deleting a host with live sessions: block it, or close the sessions first.
  - Starting a tunnel twice: the supervisor checks its state first.
  - Concurrent DB writes: one `rusqlite::Connection` behind a `Mutex` in `spawn_blocking`, with WAL on.
- **Time.** Store UTC epoch ms and show local time. Logs and history sort by integer time.
- **OS differences.**
  - macOS 15+ asks for Local Network permission the first time you reach a LAN host. Add the usage string to `Info.plist`.
  - Windows needs ConPTY (Windows 10 1809+).
  - On Linux, the WebKitGTK webgl renderer can misbehave. Fall back to the canvas renderer automatically.
- **Exec channels on network gear.** OS detection, history import and autocomplete open extra exec channels, which break some routers and switches (recon source 13). Make each one a setting, on by default but skipped when it fails.
- **Search.** The data set is small (hundreds of hosts), so in-memory fuzzy search in the UI is enough. No full-text index is needed.
- **Not applicable:** multi-tenancy, email deliverability, webhooks/idempotency, rate limits, GDPR deletion (local data only; "delete vault" removes the file).

## Build order

Each milestone ends with something you can run (`pnpm tauri dev`).

**M0: Scaffold (½ week).** Nuxt 4 SPA + Tauri 2 + shadcn-vue + Tailwind + Pinia, pnpm, Node 24.12.0. Rust modules stubbed, `rusqlite_migration` running `0001_init`, typed TS bindings generated from Rust, `AppError`, CI build for macOS.

**M1: Vertical slice, the core loop (1–1½ weeks).** Screens S24 → S02 (flat list) → S03 (address, port, username, password only) → S18.
- Tables: `vaults`, `secrets`, `hosts`, `connection_profiles`, `known_hosts`, `connection_history`, `settings`.
- Commands: `vault_*`, `hosts_tree`, `host_create/update/delete/get`, `session_open/write/resize/respond/close/reconnect`, `known_hosts_list`.
- Proves: unlock, save a host, connect with a password, trust the host key, type in a real shell, disconnect and reconnect, and the "key changed" block (F01, F11, F12).

**M2: Must-haves.**
1. *Hosts:* groups, inheritance with "from group" hints, tags and filter, drag to move (S02–S04, F04).
2. *Keychain:* generate, import, passphrase prompt, identities, export to host (S05–S08, F02). Known hosts screen (S14).
3. *Navigation:* tab bar with status dots, New Tab, Cmd+K palette, many sessions, Settings basics with font, theme and scrollback (S01, S16, S17, S23; F03).
4. *Connection extras:* jump chains (F05), startup snippet, clear error states.
5. *Snippets:* CRUD, packages, run from the side panel (S11, S12, S20).
6. *Port forwarding:* local, remote, dynamic, start/stop, supervisor (S09, S10, F06).
7. *SFTP:* dual pane, list/mkdir/rename/delete, hidden files, typed path, drag transfers with progress (S21, S22, F07 minus edit).

**M3: Should-haves.**
- Workspaces: split, focus, broadcast, templates, drag tab onto tab (S19, F09).
- Multi-host snippet run (S13, F08).
- SFTP: edit and re-upload, conflicts, chmod.
- Env vars, proxy, agent forwarding.
- Import from ssh_config and CSV (S25, F10).
- Themes, command history, local terminal, session logs (S15).

**M4: Could-haves**, ordered by value:
1. find in terminal
2. custom shortcuts
3. certificates
4. OS icon
5. Touch ID unlock
6. Telnet
7. serial
8. Mosh
9. FIDO2
10. PuTTY / MobaXterm import
11. AI with your own key
12. self-hosted sync

**M5: Fixes from `/replica-entrepreneur`**, once it has run (the Termius complaints worth beating).

Rough size: M0–M1 about 2 weeks, M2 about 5–6 weeks, M3 about 3–4 weeks. That fits the recon's **L** estimate.

## Next step

`/replica-design`: tokens and component specs for S01–S25 in shadcn-vue + Tailwind, with your own palette and icons. Or start M0–M1 in Claude Code first: see `CLAUDE.md` in the project root.
