# Improvement plan for Dalan (from the field report, 2026-11-06)

Source: `replica/field-report-2026-11-06.md` (gap table rows 1 to 34, sections 10 to 12). Written for Mehrdad to approve, and for Claude Cowork to refine. Same rules as always: clean room (our own words, tokens and drawings), the hard rules in `CLAUDE.md` still win, and nothing ships without a hand check by Mehrdad.

## 1. What this plan tries to do

Dalan already matches the reference on the essentials and is ahead on safety. The look found a set of **everyday-feel gaps**: how a connection explains itself, how fast common actions are reached (right-click menus, quick connect), how long forms are organised, and how the terminal side panel and workspaces behave. This plan closes those gaps in five small phases, and keeps the larger questions (sync, teams, autocomplete, hardware keys) as explicit decisions.

**Principles for every item**
1. Never weaken a hard rule: secrets stay in Rust (the reference shows private keys in its UI; Dalan never will), host keys stay strict (the reference's one-click "Replace" is not an option), no editable remote scripts.
2. Reuse what exists before adding: the command palette, stores, `Sheet`, `ContextMenu` components and `useShortcutLabel` are already there.
3. Each phase ends with the usual checks plus a short hand-check list for Mehrdad, and updates `features.csv` honestly (`partial` until seen).
4. Copy is written fresh in Dalan's voice (`replica/brand.md`); drawings are our own.

## 2. Scope at a glance

| Phase | Theme | Rows from the report | Effort | Risk |
|-------|-------|----------------------|--------|------|
| P21 | Connection experience | 18, 19, 24, 27, 30 | M | Low-medium (touches the session overlay and Rust prompts) |
| P22 | Context menus and quick actions | 2, 3, 4, 22, 23, 28, 29 | M | Low |
| P23 | Host editor restructure | 1, 16, 26 | M-L | Medium (largest UI change; keep the data model) |
| P24 | Terminal side panel and workspaces | 6, 7, 21, 32, 34 | M | Low |
| P25 | Settings, copy and import polish | 10, 11, 20, 31 | S-M | Low |
| Decisions | Larger bets | 9, 12, 13, 14, 15 | L | Needs Mehrdad |
| Not doing | | 17 and the "do not copy" list | n/a | n/a |

Already done and only needing confirmation: row 25 (unread dots, P18) and row 33 (drag a tab onto a tab, P18).

Suggested order: P21, P22, P24, P23, P25. P21 and P22 give the most visible gain for the least risk. P23 is the biggest and benefits from the others being finished first.

## 3. Phases

### P21: Connection experience (S18, F01, F11, F12)

**Why:** the reference turns every connection into one readable screen with a step log, and survives drops without clicks. Dalan's overlay is correct but tells the user less when something goes wrong.

1. **Step log on the connection card** (row 18).
   - Rust already knows the steps (resolve, connect, host key, each auth method, shell). Emit them as a new `SessionEvent::Step { text }` (generated into `bindings.ts`), cap the list, keep it in memory only.
   - UI: a collapsed "Show details" list under the overlay title, a **Copy details** button, and the list opens by itself when the connection fails. No secrets in the log (no passwords, no key text; host names are fine because the user already sees them; the diagnostics redaction helper still applies when copying).
   - Acceptance: failing, unreachable and wrong-password cases each show a log that says where it stopped; Copy puts plain text on the clipboard; a test asserts no password string appears in emitted steps.
2. **"Continue once" on the first-connection question** (row 19).
   - Rust already supports trust-once (`remember: false`). Add a secondary button "Connect once" next to "Trust and connect". The changed-key card does **not** get a once option.
   - Acceptance: connect once, check that `known_hosts` has no new row; ask again next time.
3. **One "sessions dropped" card for many tabs** (row 24).
   - When two or more sessions drop within a few seconds, show a single bottom-left card: count, "Reconnect all", "Close them". Single drops keep the current per-tab overlay.
   - Acceptance: e2e test with a fake backend dropping three sessions shows one card; Reconnect all calls reconnect for each.
4. **Optional automatic retry** (row 27).
   - New setting "Reconnect automatically after a drop" (default off), exponential backoff (2, 4, 8 s, max 5 tries), stops on auth failure or a changed key, keeps scrollback (already true).
   - Acceptance: unit test of the backoff schedule; hand check by restarting a test container.
5. **Auth method choice after a failure** (row 30).
   - After "Try again" on a sign-in failure, offer a small chooser: password, key, keyboard-interactive, limited to the methods the server advertised (Rust already knows them from the none-probe added in P17; send them in the error event).
   - Acceptance: against the test container, the chooser lists only the offered methods; choosing one reruns just that method.

**Out of scope:** anything that shows a fingerprint comparison differently; the changed-key card stays as built.

### P22: Context menus and quick actions (S02, S05, S16, S21)

**Why:** the reference gives every card and tab a right-click menu with the common actions. Dalan has menus on SFTP rows and tags only.

1. **Host card menu** (row 28), using the existing `ContextMenu` component:
   Connect, Connect with SFTP, Add to workspace (submenu: New workspace plus open ones), Edit, Duplicate, Move to group (submenu), Copy address, Remove (with the existing confirmation). Show the shortcut hint beside items that have one.
   - Needs store actions that mostly exist; "Connect with SFTP" opens the SFTP tab with that host in the right pane.
2. **Tab menu** (row 29): Rename (already on double-click and F2), Close, Close others, Duplicate tab.
3. **Key card and snippet card menus** (same pattern): Edit, Install on a server, Duplicate, Remove.
4. **SFTP "Actions" button** per pane (row 23), a visible twin of the right-click menu for people who do not right-click.
5. **Quick connect from the search box** (row 2): typing `user@host[:port]` and pressing Enter connects without saving, with a small "Save this host" action afterwards. Reuses the palette's parsing.
6. **Surface Local terminal and Serial in the Hosts toolbar** (row 3).
7. **Live badge on host cards** that have an open session (row 22).
8. **Sort and filter on every section list** (row 4): reuse the Hosts controls on Keychain, Snippets, Port forwarding and Known hosts.

- Acceptance: each menu is reachable by right-click and by keyboard (the menu key / Shift+F10), follows the existing a11y rules, and has an e2e test through the fake backend.
- Risk: right-click in WKWebView needs the context menu event to reach Reka's trigger; the SFTP fix in P15 is the pattern to follow.

### P23: Host editor restructure (S03, S04, S07, F04)

**Why:** one long scroll with every option visible is heavy; the reference shows the basics first and opens each advanced block in its own panel with a back arrow, and shows a live draft card.

1. **Draft card in the list** while a new host is being edited (row 1).
2. **Address first, then label** (the label can default from the address).
3. **Drill-in sub-panels** for the advanced blocks: jump hosts, proxy, environment variables, startup snippet, agent forwarding, terminal theme, protocol-specific settings. Each opens in the same panel with a back arrow and a summary line on the main page ("Through bastion", "SOCKS5 proxy.example:1080").
4. **Visual jump chain** (row 16): a vertical list of hops ending in the target, with Add hop and Clear; reuses the existing chain data.
5. **Credential reuse hint** (row 26): when two or more hosts use identical saved credentials, show "These hosts share a login. Make an identity?" Needs a Rust query that compares secrets **without exposing them** (compare inside Rust by decrypting in memory and hashing, return only host ids; never send values to the UI).
- Acceptance: the data model and Rust commands stay as they are, apart from the credential-reuse query; all existing editor e2e tests pass; unsaved-changes guard still works; keyboard order and focus rules from `components.md` still hold.
- Risk: this is the biggest UI refactor. Do it behind the same component names, and ship the draft card and sub-panels in two commits.

### P24: Terminal side panel and workspaces (S19, S20)

1. **Side panel tabs for search, theme and font** (row 6) next to the existing Snippets, History and Ask AI. Theme tab shows previews drawn from our own tokens; font tab uses the settings added in P18.
2. **History to snippet suggestion** (row 7): after N repeated commands, a small "Save these as a snippet?" row in the History tab.
3. **Remember the last side panel tab** (row 21).
4. **Broadcast cue on panes** (row 34): a dashed frame on every pane that receives typing, in addition to the existing pill.
5. **Snippet run as a list layout** (row 32), optional: a "List" layout for a workspace (terminal list on the left, one big terminal) alongside Split and Focus; snippet runs reuse an open tab for the same snippet instead of opening a new one each time (an observed annoyance in the reference).
- Acceptance: each item has an e2e or store test; the broadcast frame uses a token (no raw colour); reduced-motion users get no animation.

### P25: Settings, copy and import polish

1. **Key generation hints** (row 31): one line under each type saying what supports it, and a note that ML-DSA exists only on servers that enable it (Dalan does not offer ML-DSA or 1024-bit RSA today; keep it that way unless Mehrdad asks).
2. **Settings visibility** (row 10): show "Reconnect automatically" (from P21), "Import shell history" (if built) and bell sound as visible switches.
3. **SFTP open-with list** (row 11): remembered app per file type, with a Rust command that opens a file with a chosen app (needs a decision on how to pick an app without giving the webview file access: the OS dialog returns a path, Rust launches it).
4. **Import review polish** (row 20): group tick-boxes (select or clear all rows from one source) and a filter box. Keep "no keys, no known_hosts by default".
- Acceptance: copy reviewed against `brand.md`; no regression in the import e2e test.

## 4. Decisions needed from Mehrdad (the larger bets)

| Row | Question | Recommendation |
|-----|----------|----------------|
| 13 | Cloud sync or team sharing? | Not now. It needs an account system and breaks the local-first promise. Revisit after P25, and then only as self-hosted sync with end-to-end encryption. |
| 9 | Shell-aware autocomplete? | Start with a cheap version: complete from history and snippets in the side panel. Skip shell integration for now (large, fragile). |
| 12 | Touch ID and FIDO2 keys? | Worth a spike for the team's macOS laptops (Touch ID via the Secure Enclave for the vault unlock first); FIDO2 later. |
| 14 | Cloud-provider host import (AWS, DigitalOcean, Azure)? | Only the providers the team uses, one at a time, with API keys held in Rust. Ask which ones. |
| 15 | One host with several protocols (Telnet block)? | No. Dalan's per-host protocol is simpler; keep it. |

## 5. Not doing (and why)

- Anything that shows keys in the interface, a one-click replace on a changed host key, plan-gated features, a default of listening on every network interface, or sign-in for a desktop client.
- Community theme popularity counts (needs a server).
- 1024-bit RSA generation.

## 6. How each phase is verified

1. The standard checks: `pnpm lint && pnpm typecheck && pnpm test`, `cargo clippy --all-targets -- -D warnings && cargo test`, the sweep, contrast, and `pnpm test:e2e`.
2. A new e2e test per user-visible change, through the fake backend in `tests/e2e/`.
3. A short hand-check list (against `scripts/test-servers.sh`) written into `replica/progress.md` and reported to Mehrdad after each phase. A phase that needs a hand check is not done until he reports back.
4. `features.csv` stays `partial` until he has seen the change work.

## 7. Open points the report could not settle

These need a hand look before or during the phases:
1. How the reference shows **transfer progress** for a large or slow transfer (localhost was too fast). Matters for the SFTP part of P22.
2. Whether its **split panes resize** by dragging, and the exact keyboard shortcuts for moving focus (the list shows an Option-Command-arrows chord). Matters for P24.
3. What its **autocomplete** actually does day to day. Matters for the autocomplete decision.
4. What the **Interactive** method chooser does against a server with a real 2FA prompt. Matters for P21.

## 8. Suggested next step

Approve the order (P21, P22, P24, P23, P25) and the recommendations in section 4, then start P21 with `go`. If Mehrdad wants fewer, smaller phases, merge P24 and P25.
