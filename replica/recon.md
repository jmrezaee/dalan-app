# Recon map: Termius (desktop: macOS / Windows / Linux)

Scope: the core loop plus SFTP. That covers the hosts and groups vault, the keychain (keys, identities, certificates), the SSH terminal with tabs, workspaces, split view and broadcast, snippets, port forwarding, known hosts, SFTP and import. Cloud sync, teams, multiplayer, Mosh and serial are listed but deferred.
For: personal use and Mehrdad's team. There is no billing, and it is single-user with one local encrypted vault.
Target stack: Nuxt 4 (SPA, `ssr: false`), TypeScript, shadcn-vue, Tailwind, pnpm, Node 24.12.0, inside a **Tauri** desktop shell. SSH, SFTP and tunnels run natively on the Tauri side and never in the webview.
Date: 2026-11-04

Method: public docs, the pricing page, the changelog and the redesign blog post only. No account was logged into. No app bundles, binaries or network traffic were inspected. `replica/screens/` is empty for now. To add reference screenshots, take them yourself from your own Termius install (check Termius's Terms of Service before using your account as a reference). Screenshots stay out of the clone.

## Sources

| # | source | URL | notes |
| --- | --- | --- | --- |
| 1 | docs index | https://docs.termius.com/llms.txt | full page list |
| 2 | docs full text | https://docs.termius.com/llms-full.txt | the most complete feature list |
| 3 | groups and tags | https://docs.termius.com/organize-and-connect-to-hosts/groups-and-tags.md | inheritance rules |
| 4 | port forwarding | https://docs.termius.com/organize-and-connect-to-hosts/port-forwarding-and-tunneling.md | L/R/D fields |
| 5 | SFTP | https://docs.termius.com/organize-and-connect-to-hosts/managing-files-with-sftp.md | dual pane, edit-and-upload |
| 6 | session logs | https://docs.termius.com/organize-and-connect-to-hosts/session-logs.md | retention, bookmarks |
| 7 | keys and certificates | https://docs.termius.com/keychain/ssh-keys-and-certificates.md | generate, import, export to host, FIDO2 |
| 8 | identities | https://docs.termius.com/keychain/identities.md | |
| 9 | sync of keys | https://docs.termius.com/keychain/sync-of-keys-and-passwords.md | deferred feature |
| 10 | snippets | https://docs.termius.com/terminal/snippets.md | packages, multi-host run, startup snippet |
| 11 | workspaces | https://docs.termius.com/terminal/workspaces.md | split, focus, broadcast, shortcuts |
| 12 | environment variables | https://docs.termius.com/terminal/environment-variables.md | SendEnv / AcceptEnv |
| 13 | can't connect | https://docs.termius.com/help-center/troubleshooting/i-cant-connect-to-a-host.md | error cases |
| 14 | pricing | https://termius.com/pricing | what is gated |
| 15 | billing | https://docs.termius.com/administration/billing-and-plans | |
| 16 | changelog | https://docs.termius.com/changelog | what changed in 2025 and 2026 |
| 17 | redesign blog | https://termius.com/blog/termius-x | horizontal tabs, Vault tab, command palette |

## Core loop

Save a server once, with its credentials, keys and inherited group settings. After that, one keystroke opens a fast, good-looking terminal or file browser on it, across many hosts at once.

Pricing tells you what matters (source 14). The free tier already includes SSH, SFTP, port forwarding, the local vault, broadcast, FIDO2, proxies, jump hosts and environment variables. Termius charges for **sync, session logs and teams**. For a personal clone, everything in the free tier is the must-have set.

## Navigation model

Since the redesign there is no left sidebar. The app uses a **horizontal tab bar** like a browser. It has a fixed **Vault** tab, an **SFTP** tab, one tab per connection or workspace, and a **+** that opens the New Tab screen. Inside the Vault tab a secondary nav switches between Hosts, Keychain, Port Forwarding, Snippets, Known Hosts and Logs. **Cmd/Ctrl+K** opens a command palette over hosts and open tabs. (Sources 16 and 17.)

## Screens

| ID | screen | route / how to reach | purpose | key components | states seen |
| --- | --- | --- | --- | --- | --- |
| S01 | App shell | always | tab bar (Vault, SFTP, sessions, +), window chrome | TabBar, Tab with status dot, IconButton | 0 sessions, many sessions (overflow), unread / needs-input / error dots |
| S02 | Hosts | `/vault/hosts` | list, search and filter hosts and groups, connect | Toolbar (New Host ▾: Group, Import), SearchInput, TagFilter, GroupCard, HostCard, grid/list toggle, ContextMenu | empty vault, filled, search no results, inside a group (breadcrumb) |
| S03 | Host editor | New Host, or right-click → Edit (side sheet) | create or edit a host | Sheet, Input, Select (group, identity), CredentialPicker, TagInput, ChainPicker, EnvVarList, ThemePicker, SnippetPicker | new, edit, inherited-value hints, validation error |
| S04 | Group editor | New Host ▾ → Group | nesting and inheritable defaults | same fields as S03, minus address | new, edit |
| S05 | Keychain | `/vault/keychain` | keys, identities, certificates | Toolbar (Key ▾ Generate / Import, Identity, Certificate, FIDO2), KeyCard, IdentityCard | empty, filled |
| S06 | Key generate / import | Keychain → Key ▾ | generate (Ed25519, ECDSA, RSA) or paste / drop / file-import | Dialog, Select, PasswordInput, Switch "save passphrase", Dropzone | generating, wrong passphrase, invalid key |
| S07 | Identity editor | Keychain → New Identity | username plus password or key | Sheet, Input, CredentialPicker | |
| S08 | Export key to host | right-click key → Export to host | append the public key to `authorized_keys` and link it | Dialog, HostPicker, Input (path, file name) | running, success, auth failed |
| S09 | Port forwarding list | `/vault/port-forwarding` | rules with start / stop | RuleCard with status, Toggle | empty, active, error (port in use) |
| S10 | Port forwarding editor | New rule | Local / Remote / Dynamic wizard or single form | Stepper, RadioGroup, HostPicker, NumberInput | per-type field sets, validation |
| S11 | Snippets | `/vault/snippets` | snippets and packages, run | SnippetCard, PackageCard, Toolbar (New ▾ Package) | empty, filled |
| S12 | Snippet editor | New Snippet | label plus script body, default targets | Sheet, CodeEditor (monospace textarea), HostMultiPicker | |
| S13 | Run snippet on hosts | Snippet → Run | pick hosts, which opens a workspace | Dialog, HostMultiPicker | missing-credential prompt |
| S14 | Known hosts | `/vault/known-hosts` | trusted host fingerprints, delete to re-trust | Table (host, key type, fingerprint) | empty, filled |
| S15 | Logs | `/vault/logs` | last session log per host, plus bookmarks | List, Bookmark toggle, comment input, vault filter | empty, filled; *should*, not must |
| S16 | New Tab | `+` / Cmd+T | recent connections, search, multi-select to make a workspace | SearchInput, RecentList with checkboxes, "Create workspace" | no history, filled |
| S17 | Command palette | Cmd/Ctrl+K, or start typing on S16 | fuzzy-jump to a host, tab or action | CommandDialog (shadcn Command) | no results |
| S18 | Terminal session | double-click host / palette | one SSH (or local) shell | xterm.js, status overlay, PromptDialog | connecting, host-key prompt (first time), host-key **changed** (blocking warning), password / passphrase prompt, connected, disconnected (Reconnect / Close), auth failed, unreachable, KEX failure |
| S19 | Workspace | drag one tab onto another, or S16 / S13 | several sessions in Split (up to 16) or Focus mode, broadcast | ResizablePanels, PaneHeader, Broadcast toggle, save-dot | split, focus, broadcasting (frame on main pane), unsaved changes |
| S20 | Terminal side panel | Cmd/Ctrl+. | snippets, history, theme for the current session | Sheet with Tabs, list, theme swatches | |
| S21 | SFTP | SFTP tab | two panes, each local or a host, transfers | PathBar (click to type), FileTable, Actions menu (hidden files, new folder, refresh), ContextMenu (open, open with, copy to other side, rename, delete, permissions), Dropzone | local default, pick-host empty pane, loading, permission denied, SFTP subsystem disabled |
| S22 | Transfers | drawer in S21 | queue, progress, conflicts | ProgressRow, ConflictDialog (replace / keep both / skip) | running, conflict, failed, done |
| S23 | Settings | Cmd/Ctrl+, | Appearance, Terminal (font, cursor, scrollback, PQ KEX, OS detect), Shortcuts, SFTP (open-with apps), Logs, Security (app lock), Local terminal path | Tabs / settings nav, Form controls, ShortcutRecorder | |
| S24 | Unlock / first run | app start | create or enter the master password that encrypts the local vault | Card, PasswordInput | first run, locked, wrong password |
| S25 | Import | New Host ▾ → Import | `~/.ssh/config` (+ keys, known_hosts), CSV | Dialog, file picker, preview Table with checkboxes | parse warnings (ProxyCommand ignored), duplicates |

Count: **25 screens**.

## Flows

```
F01 Add a host and connect with a password
    S02 -> S03 (address, user, password, Save) -> double-click host -> S18 host-key prompt -> Trust -> shell
    happy path clicks: 5 + typing
    edge: unreachable host, wrong password (retry prompt, don't save), port not 22, IPv6 literal

F02 Generate a key, push it to a server, connect key-only
    S05 -> S06 Generate (Ed25519) -> S08 Export to host -> S03 credential now = key -> S18
    happy path clicks: 6
    edge: authorized_keys in a non-default path, passphrase not saved (prompt every connect), server rejects key type

F03 Quick connect from anywhere
    Cmd+K -> type "prod" -> Enter -> S18
    happy path: 1 shortcut + Enter   <- the number to beat
    edge: several matches, host already open (focus the existing tab or open a new one?)

F04 Organise with groups and inheritance
    S02 New ▾ Group -> S04 (label, identity, theme, jump host) -> drag hosts into group
    -> a host with empty username now resolves to the group identity
    happy path clicks: 5
    edge: nested groups override one field each; deleting a group with hosts (move to parent)

F05 Connect through a jump host / chain
    S03 Host chain: [bastion] -> save -> connect -> S18
    edge: bastion needs its own key; chain of 3; a bastion host-key prompt appears before the target's

F06 Local port forward to a database
    S09 New -> S10 Local (local 5432 -> via host -> 127.0.0.1:5432) -> save -> double-click start
    happy path clicks: 6
    edge: port in use, remote forward needs GatewayPorts, tunnel dies when SSH drops (auto-restart?)

F07 Move files with SFTP and edit one in place
    SFTP tab -> left pane local, right pane pick host -> drag file across -> S22 progress
    -> double-click remote file -> opens in local editor -> on save, prompt "upload changes?"
    happy path clicks: 4
    edge: overwrite conflict, permission denied, huge file / cancel, SFTP subsystem disabled, symlinks

F08 Save a snippet and run it on several hosts
    S11 New -> S12 (label, script) -> Run -> S13 pick 3 hosts -> S19 workspace with 3 panes running it
    edge: one host fails auth (red dot), host needs a sudo password (yellow dot)

F09 Multi-server workspace with broadcast
    S16 select 4 recents -> Create workspace -> S19 split -> Cmd+B broadcast -> type -> Cmd+S save template
    edge: a pane disconnects mid-broadcast; reopen the template next day (reconnect all)

F10 First run and import existing setup
    S24 set master password -> S02 empty state "Import" -> S25 ~/.ssh/config -> preview -> Import
    edge: ProxyCommand lines, Include directives, Host * wildcards (skip or make a group default)

F11 Connection drops
    S18 disconnected overlay -> Reconnect (Enter) -> same tab resumes a new session
    edge: laptop sleep, network change, keepalive timeout

F12 Host key changed
    S18 blocking warning showing old vs new fingerprint -> Cancel (default) or Replace in S14 and connect
```

## Components

| component | variants | states | used on |
| --- | --- | --- | --- |
| TabBar / Tab | fixed (Vault, SFTP), session, workspace | active, hover, status dot (green unread, yellow input, red error), unsaved dot, drag-over | S01 |
| HostCard | grid, list | default, hover, selected, connecting, OS icon | S02, S16, S17 |
| GroupCard | grid, list | default, drop-target | S02 |
| CredentialPicker | password, key, identity, certificate | inherited (ghost text "from group X"), empty | S03, S04, S07 |
| TagInput | | suggestions, chip remove | S03 |
| HostPicker / HostMultiPicker | single, multi | search, empty | S08, S10, S13, S21 |
| Sheet (side editor) | right | dirty, saving, validation | S03, S04, S07, S12 |
| Dialog | default, destructive confirm, prompt | | many |
| CommandDialog | | results, empty | S17 |
| TerminalView (xterm.js) | session, log replay (read-only) | connecting, live, disconnected overlay | S18, S19, S15 |
| StatusOverlay | connecting, error, disconnected, host-key | | S18 |
| PaneHeader | | main-input frame, broadcasting | S19 |
| FileTable | local, remote | loading, empty folder, selected, drag source, drop target, permission denied | S21 |
| PathBar | breadcrumb, edit mode | | S21 |
| ProgressRow | upload, download | running, paused, failed, done | S22 |
| RuleCard | local, remote, dynamic | stopped, starting, active, error | S09 |
| Toolbar with split button | New ▾ | | S02, S05, S11 |
| ContextMenu | host, group, key, snippet, file, tab | | many |
| EmptyState | per screen, with primary action | | S02, S05, S09, S11, S14, S15 |
| Toast | success, error | | global |
| ShortcutRecorder | | recording, conflict | S23 |
| Form primitives | Input, PasswordInput (reveal), NumberInput, Select, Switch, RadioGroup, Textarea, Stepper | default, focus, error, disabled | all |

## Inferred data model

```
Vault       id, name, kind (local)            -- one local vault in the clone; kept as a column for future sync/teams
            evidence: source 1 "Learn about vaults"; confidence: high (concept), guess (clone needs only one)

Group       id, vault_id, parent_id?, label, + all InheritableSettings
            evidence: S04, source 3; confidence: high

Host        id, vault_id, group_id?, label, address, os_icon?, + InheritableSettings
            ssh_port, sftp_enabled, telnet_port? (could), mosh? (deferred)
            evidence: S03, sources 2, 3; confidence: high

InheritableSettings (columns on Host and Group; null = inherit up the parent chain, per field)
            username?, identity_id?, password_secret_id?, key_id?, chain (ordered host_ids)?,
            proxy {type socks5|http, host, port, user, secret_id}?, agent_forwarding?,
            startup_snippet_id?, env_vars [{key, value}]?, terminal_theme?, font_size?, charset?
            evidence: source 3 "inherits any property it does not override"; confidence: high (rule), medium (exact field list)

Tag         id, name;  HostTag (host_id, tag_id)
            evidence: source 3; confidence: high

Identity    id, vault_id, label, username, password_secret_id?, key_id?
            evidence: source 8; confidence: high

Key         id, vault_id, label, type (ed25519|ecdsa|rsa), public_key, private_key_secret_id,
            passphrase_secret_id?, certificate? (text), fingerprint, created_at
            evidence: S06, source 7; confidence: high

Secret      id, ciphertext, nonce   -- every password, passphrase and private key; encrypted with a key derived from the master password (or held in the OS keychain)
            evidence: none (design choice); confidence: guess

KnownHost   id, host_pattern (host:port), key_type, public_key, fingerprint_sha256, added_at
            evidence: S14, source 17; confidence: high

PortForward id, vault_id, label, type (local|remote|dynamic), host_id, bind_address, bind_port,
            dest_address?, dest_port?, autostart?
            evidence: S10, source 4; confidence: high

SnippetPackage id, vault_id, label
Snippet     id, vault_id, package_id?, label, script, default_target_host_ids[]
            evidence: S11, S12, source 10; confidence: high

WorkspaceTemplate id, name, layout (split|focus + pane tree json), panes [{host_id|local, title, cwd?, command?}]
            evidence: S19, source 11; confidence: medium

SessionLog  id, host_id, started_at, ended_at, data (encrypted blob or file), bookmarked, comment
            evidence: S15, source 6; confidence: high (one "recent" per host, plus bookmarks)

CommandHistory id, host_id, command, ran_at   -- for the side panel and autocomplete
            evidence: S20; confidence: medium

Settings    key/value json (appearance, terminal, shortcuts, sftp apps, security)
            evidence: S23; confidence: high
```

Relationships: Vault 1-n {Group, Host, Identity, Key, Snippet, PortForward}. Group 1-n Group (tree). Group 1-n Host. Host n-n Tag. Host/Group n-1 Identity, n-1 Key. Identity n-1 Key. Host n-n Host through the chain (ordered). PortForward n-1 Host. Snippet n-1 SnippetPackage. Host 1-n SessionLog, 1-n CommandHistory.

Runtime-only (not stored): Session {id, host_id, state, channel}, Tunnel {rule_id, state}, Transfer {id, src, dst, bytes, total, state}.

## Feature matrix

See `features.csv`. Must: 33, should: 18, could: 13, skip: 9.

## Out of scope (cannot or should not be cloned)

- **Termius cloud sync, accounts and end-to-end encrypted cloud vaults.** These are their backend and service. A self-built sync can come later. It is listed as `could`.
- **Team vaults, seats, permissions, SSO, SOC 2, real-time terminal multiplayer.** These are their backend and the business plan. They are out of scope for a personal clone.
- **SSH.id passkeys service.** This is their identity service. Use local keys, FIDO2 and the OS agent instead.
- **Termius AI (command generation, AI snippet writing).** This is their model and service. If wanted, wire in your own API key later (`could`).
- **Cloud provider host sync (AWS, DigitalOcean, Azure).** This is possible through official APIs, but it is not core (`could`).
- **Their brand, icon, theme names, illustrations and copy.** Use your own name, palette and icons (see `/replica-brand`).
- **App store distribution and the mobile apps.** Desktop only.

## Size

Screens 25, flows 12, entities 14.

Hard parts:
1. **The SSH engine and terminal pipe.** This means native SSH in Tauri: Rust `russh` and `russh-sftp`, or a Node `ssh2` sidecar. It must stream PTY bytes to xterm.js over Tauri events with low latency, plus handle resize, keepalive, reconnect, jump-host chains, agent forwarding and the host-key verification UX.
2. **Secrets and keys.** That covers an encrypted local vault (master password with Argon2 and XChaCha, or the OS keychain), key generation and parsing (OpenSSH, PEM, PuTTY .ppk), passphrases, certificates, and FIDO2 `sk-` keys.
3. **SFTP and tunnels as long-lived native tasks.** This includes a dual-pane browser, a transfer queue with progress, conflicts and cancel, edit-and-watch-and-reupload, and port-forward listeners whose lifecycle is tied to SSH connections that drop.

Honourable mentions: workspaces with a resizable split tree and broadcast input; inheritance resolution across nested groups; parsing `~/.ssh/config`.

Size: **L (about a quarter)** for the full must and should slice. A usable MVP (hosts, groups, keychain, terminal with tabs, palette, known hosts, basic SFTP) is **M (a few weeks)**. It will not be a perfect Termius. The aim is to match the core loop and beat it on F03 (connect in one shortcut).

Next step: `/replica-architect`. It picks the SSH engine (Rust `russh` vs Node `ssh2` sidecar), turns this model into a SQLite schema, and plans the Tauri commands and events.
