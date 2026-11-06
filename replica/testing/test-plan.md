# Dalan test plan

This is the implementation guide for Claude Code. It covers the full test strategy for Dalan, the files to create, the conventions, the coverage matrix (every screen S01–S25 and flow F01–F12) and the two commands Mehrdad runs to watch the app being tested:

```bash
pnpm walk        # opens a browser and walks through the whole app, step by step, with captions; then opens a report
pnpm walk:app    # the same walk on the REAL app (real Rust backend, real SSH servers) inside a Linux container, recorded as video
```

The plan is built as phases T1 to T7 (§14). Each phase ends with its acceptance checks green.

---

## 1. Principles

1. **Tests are the definition of done.**
   - A row in `replica/features.csv` may say `yes` only when an automated test covers it, or when a dated hand check is recorded in `replica/testing/hand-checks.md`.
   - Add a `test` column to `features.csv` naming the covering test IDs (§13).
2. **Test each thing at the lowest layer that can prove it.** Rust logic belongs in Rust tests, UI states in browser tests, real integration in the real-app tests.
3. **Deterministic, or it doesn't merge.**
   - No real time: use the fake clock.
   - No network outside the local test servers.
   - No random IDs in assertions.
   - Animations off. Bundled fonts only.
4. **No real secrets.** Test keys and passwords are generated per run into a temporary folder. Nothing from Mehrdad's machine is ever read: no `~/.ssh`, no real known_hosts, no Keychain.
5. **Hard rules are tests.** Every rule in `CLAUDE.md` has at least one test that fails if the rule is broken (§9).
6. **Fast by default.**
   - `pnpm test` plus `cargo test` run in under 3 minutes.
   - The browser suite runs in under 6 minutes.
   - The real-app suite runs nightly, before a release, and on demand.

## 2. Layers

| # | layer | tool | runs against | command | when |
| --- | --- | --- | --- | --- | --- |
| L1 | Rust unit | `cargo test` | pure functions (parsers, resolve, crypto, history tracker, diagnostics redaction) | `cargo test` | every push |
| L2 | Rust integration | `cargo test` + in-process `russh` server + temp SQLite | sessions, auth, host keys, jump hops, tunnels, SFTP, logs, lock, migrations | `cargo test` | every push |
| L3 | Rust vs real OpenSSH | `cargo test -- --ignored` + Docker test servers | the same flows against real sshd, squid, dante, telnetd, mosh-server | `pnpm test:servers:rust` | CI (service containers), on demand |
| L4 | TS unit and stores | Vitest | utils, Pinia stores with a mocked `useTauri` | `pnpm test` | every push |
| L5 | Component | Vitest + `@nuxt/test-utils` (`mountSuspended`) + happy-dom | single components and their states | `pnpm test` | every push |
| L6 | Browser end-to-end | `@playwright/test` + built SPA + **fake backend** | every screen and flow, keyboard, a11y | `pnpm test:e2e` | every push |
| L7 | Visual regression | `@playwright/test` `toHaveScreenshot` | every screen state, dark and light | `pnpm test:visual` | every push (Linux only) |
| L8 | Walkthrough | `@playwright/test` headed, captions, gallery | one long story through the whole app | `pnpm walk` | on demand, and in CI headless |
| L9 | Real app end-to-end | `tauri-driver` + WebdriverIO + Xvfb in Docker + Docker test servers | the real binary and Rust backend | `pnpm test:app`, `pnpm walk:app` | nightly, before release, on demand |
| L10 | Hand checks | `replica/testing/hand-checks.md` | macOS-only behaviour (§10) | — | before each release |

**Why the real app runs in Linux:** `tauri-driver` (WebDriver for Tauri) supports Linux and Windows only. There is no WebDriver for WKWebView on macOS. So L9 runs the real Dalan binary on Linux (WebKitGTK) in a container, which Docker Desktop runs fine on the Mac. Behaviour specific to the macOS webview goes to L10.

## 3. Tooling changes (T1)

1. **Move the browser tests to the `@playwright/test` runner.**
   - Today they run in Vitest using the `playwright` library (`tests/e2e/*.e2e.ts`, `vitest.e2e.config.ts`). Move them to `tests/e2e/**/*.spec.ts`.
   - Keep `fakeBackend.ts` and `harness.ts` as fixtures.
   - The runner gives retries, traces, videos, an HTML report, `--headed`, `--ui`, `toHaveScreenshot` and projects.
2. **Add packages:**
   - `@playwright/test`, pinned to the same version as `playwright`
   - `@axe-core/playwright`
   - `@nuxt/test-utils`, `@vue/test-utils`, `happy-dom`
   - for L9: `@wdio/cli`, `@wdio/local-runner`, `@wdio/mocha-framework`, `@wdio/spec-reporter`, `webdriverio`
3. **`playwright.config.ts`:**

   ```ts
   import { defineConfig, devices } from '@playwright/test'
   export default defineConfig({
     testDir: 'tests',
     outputDir: 'test-results',
     fullyParallel: true,
     retries: process.env.CI ? 2 : 0,
     reporter: [['list'], ['html', { open: 'never', outputFolder: 'playwright-report' }]],
     use: {
       baseURL: 'http://127.0.0.1:4310',
       viewport: { width: 1280, height: 800 },
       trace: 'retain-on-failure',
       video: 'retain-on-failure',
       screenshot: 'only-on-failure',
       colorScheme: 'dark',
       locale: 'en-CA',
       timezoneId: 'America/Toronto',
     },
     webServer: { command: 'node tests/support/serve.mjs --port 4310', url: 'http://127.0.0.1:4310', reuseExistingServer: !process.env.CI },
     expect: { toHaveScreenshot: { maxDiffPixelRatio: 0.01, animations: 'disabled', caret: 'hide' } },
     projects: [
       { name: 'e2e', testDir: 'tests/e2e', use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 } } },
       { name: 'e2e-light', testDir: 'tests/e2e', grep: /@theme/, use: { colorScheme: 'light' } },
       { name: 'visual', testDir: 'tests/visual', use: { viewport: { width: 1280, height: 800 } } },
       { name: 'walk', testDir: 'tests/walk', timeout: 15 * 60_000, use: { video: 'on', trace: 'on' } },
     ],
   })
   ```

   - `tests/support/serve.mjs` is today's `serveApp()`: it serves `.output/public` with an SPA fallback. It runs `pnpm build` only when `.output/public` is missing or older than `app/`.
   - Use Playwright's own Chromium everywhere, not `channel: 'chrome'`, so screenshots match on every machine. `pnpm exec playwright install chromium` goes in `postinstall` for the dev machine and in CI.
4. **Scripts in `package.json`:**

   ```json
   "test": "vitest run",
   "test:e2e": "playwright test --project=e2e --project=e2e-light",
   "test:visual": "playwright test --project=visual",
   "test:visual:update": "playwright test --project=visual --update-snapshots",
   "walk": "node tests/walk/run.mjs",
   "walk:ci": "playwright test --project=walk",
   "test:servers": "scripts/test-servers.sh",
   "test:servers:rust": "scripts/test-servers.sh up && cd src-tauri && cargo test -- --ignored",
   "test:app": "node tests/app/run.mjs",
   "walk:app": "node tests/app/run.mjs --walk",
   "test:all": "pnpm lint && pnpm typecheck && pnpm test && (cd src-tauri && cargo test) && pnpm test:e2e && pnpm test:visual"
   ```

5. **Screenshots are taken on Linux only.**
   - Baselines live in `tests/visual/__screenshots__/` and are made in the pinned Playwright Docker image (`mcr.microsoft.com/playwright:v<same version>-noble`).
   - That way the Mac and CI produce the same pixels. `pnpm test:visual` runs inside that image through `scripts/in-playwright-docker.sh` when the machine is not Linux.

## 4. The fake backend (T1)

The browser layers (L6 to L8) run the real Nuxt app with a **fake Rust backend** installed through Tauri's IPC hooks. Today `tests/e2e/fakeBackend.ts` has 114 lines and fakes only hosts and sessions. It must become a complete, typed stand-in.

### 4.1 Structure

```
tests/fake/
  index.ts          installFakeBackend(page, scenario): injects the backend, exposes window.__fake
  backend.ts        runs in the page; the command table
  model.ts          in-memory vault model (types imported from app/types/bindings.ts)
  commands/         one file per area: vault, hosts, groups, keys, identities, known-hosts,
                    sessions, snippets, forwards, sftp, transfers, edit, workspaces, logs,
                    history, settings, import, ai, release, serial, themes, env, proxy, agent
  scenarios/        seed data: empty.ts, team.ts (the populated vault), locked.ts, light.ts
  scripts/          scripted servers: sessions that print believable output (see 4.3)
  contract.spec.ts  fails if a command in bindings.ts has no fake
```

### 4.2 Rules

1. **Typed from the bindings.**
   - The command table is `Record<keyof typeof commands, Handler>`, using the names from `app/types/bindings.ts`. A new Rust command makes `pnpm typecheck` fail until it has a fake.
   - `contract.spec.ts` also checks at runtime that every command in the bindings answers.
   - Answers use the generated types, so a changed Rust type breaks the fake at compile time.
2. **Errors use the real shape and codes:** `{ code, message }`, with the codes from `architecture.md`.
3. **Secrets behave like Rust.** The fake never returns a secret, only flags. The rule test in §9 scans every recorded answer.
4. **Control from the test**, through `window.__fake` (wrapped in `tests/fake/index.ts` helpers):

   ```ts
   fake.session(id).state('ready' | 'disconnected' | ...)
   fake.session(id).prompt({ kind: 'host_key_new', fingerprint })   // returns when the UI answers
   fake.session(id).output('text with \x1b[32mANSI\x1b[0m')       // raw bytes to the terminal
   fake.session(id).runScript('ubuntu-login')                       // play a scripted server (4.3)
   fake.session(id).error('auth_failed', 'web-2 rejected the key "deploy".')
   fake.transfer(id).progress(0.42) / conflict(item) / done()
   fake.tunnel(id).state('active' | 'reconnecting' | 'error', message?)
   fake.dropAll()                                                   // every session disconnects (F11)
   fake.rotateHostKey(hostId, { type?: 'ecdsa' })                   // the next connect gets a changed key (F12)
   fake.calls('session_write')                                      // what the UI sent
   fake.failNext('host_save', { code: 'validation', message: '...' })
   ```

5. **Time is controlled.** Install `page.clock` before `goto`, so auto-lock, the reconnect backoff, toasts and "2 min ago" text are deterministic.
6. **Scenarios are data, not code paths.** `scenarios/team.ts` holds:
   - 3 groups (Production with a bastion jump and an identity, Staging, Home lab), one of them nested
   - 9 hosts across SSH, Telnet, serial and Mosh; 4 tags
   - 4 keys (Ed25519 with a saved passphrase, one with a certificate, RSA asking every time, an unused ECDSA) and 2 identities
   - 5 snippets in 2 packages
   - 4 tunnels (local active, SOCKS reconnecting, remote error, local stopped)
   - 6 known hosts, 3 logs (1 bookmarked), 1 workspace template
   - command history, and settings with logs on
   - every timestamp relative to the fixed clock

### 4.3 Scripted servers

`tests/fake/scripts/*.ts` play believable terminal sessions, timed by the fake clock, so the walkthrough and the screenshots show a real-looking terminal:

- `ubuntu-login`: banner, a prompt, `git status`, `ls -la` with colours, then the prompt.
- `nginx-status`: `systemctl status` with green `active`.
- `apt-update`: progress lines, ending with "N packages can be upgraded" (for workspace broadcast).
- `sudo-prompt`: prints `[sudo] password for deploy:` (for the history filter and the yellow dot).
- `flood`: 5 MB of output in 64 KB chunks (for backpressure and responsiveness).
- `vim`: alternate screen, then exit (for the alternate buffer).

## 5. Selectors and test hooks

1. **Prefer roles and labels:** `getByRole('button', { name: 'Trust and connect' })`, `getByLabel('Address')`. That also tests accessibility.
2. **Use `data-testid` only where there's no accessible name or role.** Add these, and no others without a reason:
   - `app-tabbar`, `tab-<sessionKey>`, `tab-dot-<sessionKey>`
   - `terminal-<sessionKey>` (the xterm container), `pane-<sessionKey>`
   - `host-card-<hostId>`, `group-card-<groupId>`
   - `sftp-pane-left`, `sftp-pane-right`, `file-row-<name>`
   - `transfer-row-<id>`, `rule-card-<id>`, `toast`, `status-overlay`
3. **Read terminal text** through a helper that reads xterm's buffer (`term.buffer.active`, exposed in dev and test builds only as `window.__dalanTerm[key]`). Never screenshot the canvas to read text.
4. **Strip the hooks** from production builds through the same `pages:extend` / define flag that removes `/design`. `__dalanTerm` and the test IDs must not ship. Add a test that greps `.output` for `__dalanTerm`.

## 6. Coverage matrix: flows

Each row lists the tests to write. **E** = browser e2e (L6), **R** = Rust integration (L2/L3), **A** = real-app (L9), **V** = visual (L7). Test IDs follow `<layer>-<flow>-<n>`.

| flow | E (browser, fake backend) | R (Rust) | A (real app + Docker servers) |
| --- | --- | --- | --- |
| **F01** add a host, connect | Add host with address only and the label defaults (P23). Draft card appears and disappears. Save, then the card shows. Connect, then host-key prompt, "Trust and connect", then terminal output. "Connect once" asks again next time. Wrong password, then "Sign-in failed" and "Try another way" chips. Unreachable, then the step log opens and "Copy details" copies text. | Trust on first use saves; connect once doesn't; password, key and keyboard-interactive; `AuthMethods`; step events hold no secret | Server a: add, connect, trust, `echo ok`, see `ok` |
| **F02** key to server | Generate Ed25519, then the card. Install on a server, with the progress list and success. Import by paste with passphrase (wrong, then right). Certificate attach, refuse, expire. | Generate, import, encrypted key, export appends to `authorized_keys` with 700/600, certificate sign-in | Generate, install on server a, connect key-only to server b |
| **F03** quick connect | ⌘K, type, Enter opens a session. New Tab `user@host:port` with dedupe and port match (P27 fix). Shortcut labels follow a custom rebind. | — | ⌘K to server a |
| **F04** groups and inheritance | Group with a username, identity and jump; the host shows "from Production" hints; override, then "Use inherited"; drag a host onto a group or breadcrumb; delete a group and its children move up | `groups::resolve` per field and source; cycle refused; delete moves children | Inherited password signs in to server a |
| **F05** jump hosts | Chain editor route; hop prompts name the hop; "via bastion" line | Hop key changed blocks; forced method applies to the **target** hop (P27 fix); hop host can't be deleted | Server c (inner) through a |
| **F06** tunnels | Wizard 3 steps and skip; port-in-use error with a suggestion; card states active, reconnecting, error, stopped; Enter toggles | Local, remote, SOCKS; backoff 1 s to 30 s; permanent errors stop; autostart asks nothing | Local `-L` to a container web server and `curl` it; SOCKS with `curl --socks5`; remote with GatewayPorts |
| **F07** SFTP | Two panes; typed path resolves `~` and `..` (realpath); hidden files; mkdir, rename (F2), delete confirm, chmod; drag copy with progress; conflict (replace, keep both, skip, apply to all); cancel; "no access" and "SFTP isn't enabled" states; 600-row folder scrolls (virtual); edit in default app, then the upload prompt; executable types refused (P27) | Recursive copy, cancel removes the partial file, `remote_changed`, edit refuses over 50 MB, permissions | Upload 20 MB, cancel half way, the partial file is gone; edit a file and upload |
| **F08** snippet on hosts | Run on 3 hosts opens a workspace; "asks for password" note; history "Save as snippet" | Startup snippet runs once ready | Run `hostname` on a and b |
| **F09** workspace and broadcast | ⌘D, ⌘⇧D, resize handle, split and focus, broadcast pill and frame, skipped panes listed, save template, reopen | — | Broadcast `echo x` to 2 panes |
| **F10** first run and import | Create vault (strength meter, mismatch, 8-char message clears when fixed: P27), unlock, wrong password and cooldown, caps lock; import SSH config, CSV, PuTTY, MobaXterm with filter, select all, group boxes, duplicates unticked, ProxyCommand warning, unticked jump host kept or reported (P27) | Parsers (samples in `replica/smoke/`), apply in one transaction | Fresh vault, import `replica/smoke/ssh_config_sample` |
| **F11** connection drops | `fake.dropAll()` shows one "N dropped" card; Reconnect all; auto-reconnect schedule 2/4/8/16/30 s on the fake clock; **`exit` does not reconnect** (P27 regression test); a wrong password stops retries | Keepalive marks disconnected; reconnect on the same tab | `docker restart` a, then the card, then Reconnect all |
| **F12** identity changed | Changed key gives a blocking overlay with Close focused; Replace needs the typed host name; key-type switch blocks too; known-hosts list "Replace saved key" banner | Rust refuses replace without `confirm_replace`; key-type switch is `Changed` | `scripts/test-servers.sh rotate a`, then the block, then replace, then connect |

## 7. Coverage matrix: screens and states

For every screen, browser tests (L6) check behaviour and visual tests (L7) capture each listed state **in dark and in light**. Screenshot names: `<screen>-<state>-<theme>.png`.

| screen | states to capture and assert |
| --- | --- |
| S01 app shell and tab bar | no sessions; 6 tabs with overflow; dots: unread (green), waiting (yellow, pulse off), error (red), idle; dragging; drop target "Drop to split"; renamed tab; workspace tab with unsaved dot |
| S02 hosts | empty vault; grid; list; search with no results; tag filter; inside a nested group with breadcrumb; draft card; live dot; context menu open; "…" menu (first click opens it: P27) |
| S03 host editor | new; edit with inherited hints; validation error (port 70000); "More" sub-pages: jump route, proxy, env (inherit, merge, replace), agent warning, on connect, theme; unsaved dot; Telnet, serial and Mosh variants; shared-login note (after the P27 change) |
| S04 group editor | new; nested parent select; inherited values from the parent |
| S05 keychain | empty; keys with badges (ED25519, RSA, CERT, expires soon); identities; search and sort |
| S06 to S08 key dialogs | generate (each type hint, ML-DSA note); import (paste, file, wrong passphrase); new identity; install key (progress, success, failure) |
| S09, S10 tunnels | list with 4 states; wizard steps 1 to 3; skip to form; port in use |
| S11, S12 snippets | packages; editor; delete confirm; move by drag (`snippets_move`: P27) |
| S13 run on hosts | multi-select with group "Select all" |
| S14 known hosts | list; changed-key banner; forget confirm |
| S15 logs | empty with "Turn on"; list with a bookmark and a note; replay dialog |
| S16 new tab | no history; recents with checkboxes; "Open N as workspace"; quick-connect text |
| S17 palette | empty query; results across groups; no results |
| S18 terminal and overlays | connected with output; find bar; and each overlay: connecting with a step log, first connect, connect once, identity changed, password, passphrase, keyboard-interactive, sign-in failed with chips, unreachable with the log open, key exchange failed, disconnected, N dropped |
| S19 workspace | split 2 and 4; focus; broadcast; maximised pane |
| S20 side panel | Snippets, History (with the suggestion), Theme, Font, Ask AI (no key, a suggestion, an error) |
| S21, S22 SFTP and transfers | local and server panes; drop target; hidden files; no access; SFTP disabled; conflict row; running, failed and done; context menu with "Open with" |
| S23 settings | each section (Appearance, Terminal, Shortcuts with recorder and conflict, Security, Logs, AI, About) |
| S24 unlock | first run; unlock; wrong password; cooldown countdown; caps lock |
| S25 import | each source; warnings; filter; duplicates |

## 8. The walkthrough: `pnpm walk` (T3)

### 8.1 What Mehrdad sees

1. **`pnpm walk`** opens a visible browser window, in Playwright's own Chromium at 1280×800. It plays one continuous story through the whole app at human speed. A caption bar at the top of the page says which step it is on, for example *"Step 12 of 58 · F01 · Trust the server's key"*. Each action is highlighted for a moment (a turquoise outline on the element about to be clicked or typed into).
2. Every step asserts what should be on screen. If something is wrong, the step turns red in the caption, the walk pauses for 3 seconds on the failure, then continues with the next chapter (§8.3), so one bug doesn't hide the rest.
3. **At the end it opens a report** in the browser:
   - **Gallery** (`walk-report/index.html`): every step in order, with its screenshot, flow and screen IDs, pass or fail, and the time taken. Click a step to see the dark and light shots side by side and the diff against the saved baseline.
   - The **Playwright HTML report**, with a full video and a trace you can scrub through.

### 8.2 Options

```bash
pnpm walk                       # headed, normal speed (600 ms between actions), dark, then light for the key screens
pnpm walk --fast                # headed, 100 ms
pnpm walk --headless            # no window (what CI runs as walk:ci); still makes the gallery and video
pnpm walk --theme light         # whole walk in light
pnpm walk --from F07            # start at a chapter
pnpm walk --only F11,F12        # just these chapters
pnpm walk --pause               # wait for Enter in the terminal after each chapter
pnpm walk --update              # accept current screenshots as the new baselines (review the diff first)
```

`tests/walk/run.mjs` turns these into Playwright arguments and environment (`WALK_DELAY`, `WALK_THEME`, `WALK_FROM`, `WALK_ONLY`, `WALK_PAUSE`). It runs `playwright test --project=walk`, then builds the gallery and opens it, using `open` on macOS and `xdg-open` on Linux.

### 8.3 The story (chapters and steps)

Write it in `tests/walk/walk.spec.ts` as one `test` per chapter, run in order (`test.describe.configure({ mode: 'serial' })`). Each chapter starts from the state the previous one left, kept in one page. A failed chapter is recorded, and the next chapter restores its starting state from the scenario. Each step is `step(id, caption, async () => { ... })` (§8.4).

1. **First run (F10, S24):**
   - fresh install
   - weak, then strong password (meter)
   - mismatch message, then fixed and the message clears
   - create vault
   - empty Hosts (S02 empty)
2. **Import (F10, S25):**
   - Import, then SSH config (the sample file)
   - filter, Select all, a ProxyCommand warning, a duplicate unticked
   - Import 7, then the hosts appear
3. **Organise (F04, S02 to S04):**
   - new group Production with username `deploy`
   - drag 3 hosts in, open a host to see "from Production"
   - nested group; tag filter; list view; search
4. **Keychain (F02, S05 to S08):**
   - generate Ed25519
   - import a key by paste (wrong, then right passphrase)
   - identity; attach a certificate
   - install the key on web-2 (progress, success)
5. **Connect (F01, S18):**
   - double-click web-1, then the step log, then the first-connect prompt
   - "Trust and connect", then the `ubuntu-login` script plays; find bar (⌘F)
   - side panel: snippet, history, theme change (the terminal recolours)
6. **Sign-in problems (F01, S18):**
   - db-1 with a wrong password, "Try another way"
   - keyboard-interactive code prompt
   - unreachable host with the log open, Copy details
   - key exchange failed
7. **Navigation (F03, S01, S16, S17):**
   - ⌘T New Tab, quick connect `test@10.0.0.9:2222`
   - ⌘K palette, then a host
   - tab dots: yellow from `sudo-prompt`, green after unread output, red on error
   - rename a tab, close others
8. **Jump host (F05):** inner host via bastion; the hop prompt names the bastion; the "via bastion" line.
9. **Workspace (F08, F09, S13, S19):**
   - run "Update packages" on 3 hosts, then a workspace opens
   - broadcast pill and frames; a skipped disconnected pane
   - ⌘D, ⌘⇧D, resize, focus mode, save template
10. **Drops (F11):**
    - `fake.dropAll()`, then the "3 connections dropped" card, then Reconnect all
    - auto-reconnect on: drop again, advance the clock, the tabs come back
    - `exit` in one tab, and it **stays closed**
11. **Identity changed (F12, S14):**
    - rotate db-1's key (type switch), then the blocking overlay with Close focused
    - Known hosts banner, Replace (type the name), connect
12. **Tunnels (F06, S09, S10):**
    - wizard local 5432: port in use, use 15432, start, active
    - SOCKS reconnecting; remote error with the GatewayPorts hint
13. **Files (F07, S21, S22):**
    - SFTP: local and web-1; typed path `~/../srv`
    - hidden files, mkdir, F2 rename, chmod
    - drag 2 files: progress, conflict, "Keep both"
    - cancel a big transfer; "no access" folder
    - edit in the default app, then the upload toast; a `.command` file refused
14. **Logs and history (S15, S20):** turn logs on; a session; replay; bookmark with a note; history "Save as a snippet?".
15. **Settings (S23):**
    - each section
    - rebind New tab to ⌘Y and see labels change everywhere (tab bar tooltip, palette)
    - bell on
    - AI: no key, then a fake key, then a suggestion and Insert
16. **Light theme:** switch to Light and revisit S02, S03, S18, S19 and S21 for screenshots.
17. **Lock (S24):** Lock, then the unlock screen; wrong password ×5, then the cooldown on the fake clock; unlock.

### 8.4 Step helper

`tests/walk/step.ts`:

```ts
export async function step(page: Page, info: { id: string, flow?: string, screen?: string, caption: string }, body: () => Promise<void>) {
  await test.step(`${info.id} ${info.caption}`, async () => {
    await showCaption(page, info)                 // injects/updates a fixed top bar (outside the app root, pointer-events: none)
    await body()                                  // actions use `act()` which outlines the target, waits WALK_DELAY, then acts
    await page.waitForTimeout(Number(process.env.WALK_DELAY ?? 0))
    const shot = await page.screenshot({ animations: 'disabled', caret: 'hide', mask: dynamicMasks(page) })
    await recordStep(info, shot)                  // writes test-results/walk/<id>.png + walk.json for the gallery
  })
}
```

- The caption bar and outline live in their own shadow root, so they never show in visual baselines. Hide them in screenshots with `style` injection.
- Masks cover timestamps, relative times and transfer speeds, unless the fake clock already fixes them.
- `tests/walk/gallery.mjs` builds `walk-report/index.html` from `walk.json`: a static page using the design tokens, with a left list of steps (pass/fail dot, flow and screen chips), the screenshot, and the baseline diff (`pixelmatch`) when one exists.

### 8.5 `pnpm walk:app`: the real app (T5)

The same chapters (as far as they don't need fake-only controls), run by WebdriverIO against the **real Dalan binary** and the **real Docker test servers**. Shared step definitions go through a small driver interface:

```ts
interface Driver {
  click(role: string, name: string): Promise<void>
  fill(label: string, text: string): Promise<void>
  press(keys: string): Promise<void>
  expectText(text: string | RegExp): Promise<void>
  terminalText(sessionKey?: string): Promise<string>
  screenshot(name: string): Promise<void>
  server(cmd: 'restart' | 'rotate' | 'stop', name: string): Promise<void>   // runs scripts/test-servers.sh
}
```

- Playwright implements it for `pnpm walk`. WebdriverIO implements it for `walk:app` (`$('aria/Trust and connect')` finds by accessible name).
- A chapter that needs fake-only control (the clock, `dropAll`) uses the real equivalent where one exists (`docker restart`, `rotate`). Otherwise it is skipped in `walk:app`, with the reason in the gallery.

**`tests/app/` layout:**

```
tests/app/
  Dockerfile         rust + node 24 + pnpm + webkit2gtk-4.1 + webkit2gtk-driver + xvfb + ffmpeg + tauri-driver
  compose.yml        app runner + the test servers (a, b, c, squid, dante, telnetd, mosh) on one network
  run.mjs            builds the image, starts compose, runs wdio, copies results out, opens the gallery
  wdio.conf.ts       capabilities: { 'tauri:options': { application: '/work/src-tauri/target/debug/dalan-app' } }
  specs/*.e2e.ts     L9 tests (§6 column A)
  walk.ts            the walk on the Driver interface
```

**Rules for the real-app run:**

- Build with `pnpm tauri build --debug --no-bundle`, with the test hooks flag on.
- `Xvfb :99` runs, and `ffmpeg -f x11grab` records `walk-app.mp4` for the whole run.
- Every run uses a fresh data folder: `XDG_DATA_HOME` points to a temp folder, so the vault is new.
- Server addresses come from the compose service names; the test-server script writes them to `.test-servers/servers.json`.
- Results go to `test-results/app/`: the video, screenshots, the WebdriverIO log and the gallery.

## 9. Cross-cutting suites

### 9.1 Hard rules as tests (T4)

| rule | test |
| --- | --- |
| 1. Secrets never reach JS | **Rust:** a test walks the generated command list. For each command returning a type, a compile-time list in `tests/secret_free.rs` asserts no field is named `password`, `passphrase`, `private_key`, `secret`, `api_key` or `token`. **Browser:** the fake records every answer, and a Playwright fixture fails the test if an answer contains a known test secret string. **Real app:** after the walk, search the webview's `localStorage`, `sessionStorage` and IndexedDB for the test secrets: there must be none. |
| 2. No fs, http or shell in the webview | A Vitest test reads `src-tauri/capabilities/*.json` and fails on any permission outside an allow-list (`core:default`, `dialog:allow-open`). A Rust test checks `Cargo.toml` has no `tauri-plugin-fs`, `-http` or `-shell`. |
| 3. Terminal data is bytes | Rust: a binary payload (invalid UTF-8, split multi-byte characters) arrives byte-identical through `SessionIo`. Browser: a split emoji renders correctly. |
| 4. Tabs keep xterm mounted | Browser: write a marker into tab 1, switch tabs 10 times and open Settings, then the marker is still in tab 1's buffer and the xterm instance id is unchanged. |
| 5. Host keys always verified | R + E + A for unknown, changed and key-type switch; no setting named "accept all" (grep test); tunnel autostart refuses unknown keys. |
| 6. Generated types | CI step `git diff --exit-code app/types/bindings.ts` after the bindings build (exists); a lint rule bans hand-written interfaces that duplicate binding names. |
| 7. Error shape | Rust: every `AppError` variant serialises to exactly `{code,message}`. Browser: an unknown code still shows a friendly fallback. |
| 8. Clean room | `sweep.py` in CI (exists), and the sweep also runs on `.output` and `test-results/walk` captions. |

### 9.2 Security

- **AI address:** `localhost.evil.com`, `127.0.0.1.nip.io`, `http://[::1]`, `http://0x7f.1`, userinfo tricks (`https://a@b`) and redirects are refused (Rust).
- **CSP:** a browser test loads the built `index.html` with the production CSP header and asserts no CSP violations in the console across the walk.
- **Executable files:** Open with / Edit refuses `.command .app .desktop .jar .exe .bat .cmd .ps1 .sh(+x) .terminal .scpt .pkg .dmg`, case-insensitive (Rust, P27).
- **Shared logins (P27):** asking about one host with a guessed password reveals nothing (Rust).
- **Diagnostics:** redaction covers IPv4, IPv6, `user@host`, host labels and the home path; the vault-locked case is covered.
- **Owner-only files:** vault, logs, edit-tmp (exists; keep).
- **Cooldown:** survives a restart, if P27 makes it persistent. Otherwise document why not.

### 9.3 Accessibility (T4)

- `@axe-core/playwright` runs on every screen state in §7: zero serious or critical violations. Allowed exceptions go in `tests/a11y/allow.ts`, each with a reason.
- **Keyboard-only walk:** a chapter of the e2e suite does F01 and F03 with the keyboard only (Tab, Enter, Esc, arrows). Focus is always visible (screenshot plus an assertion that the focused element has the ring).
- **Overlays:** focus moves into them, the terminal is `inert`, and Esc picks the safe action.

### 9.4 Performance and robustness

- **Flood:** `flood` (5 MB) through the fake and the real app. The UI stays responsive (a click on another tab responds in under 200 ms during the flood). Rust shows reading paused at 1 MB behind (exists; keep).
- **Big lists:** 2,000 hosts in the scenario; the Hosts grid and palette search take under 100 ms per keystroke (performance marks in the test). A 5,000-entry SFTP folder scrolls (virtualised).
- **Long run** (nightly, real app): 30 minutes with 10 sessions printing every second. Memory (`/proc/<pid>/status` RSS) grows by less than 150 MB.

### 9.5 Migrations

- Keep the populated-0001 upgrade test.
- Add fixture databases `tests/fixtures/db/v0001.db` to `v0005.db`. Each opens, migrates to the latest version and keeps its data, then the app commands read it.
- A new migration must add its fixture.

## 10. What stays manual (L10)

`replica/testing/hand-checks.md` (T6) replaces the long smoke list with **only what automation can't cover**. Each item has steps, the expected result, and a line for the date, the result and initials.

- **The macOS window (WKWebView):** right-click menus, drag and drop (tabs, hosts, files from Finder), IME and Persian keyboard input in the terminal, ⌘ shortcuts not swallowed, copy on select to the macOS clipboard.
- **macOS integration:** single instance with `open -n`; "Open with…" uses the native panel starting in /Applications; files open in the chosen app; the system SSH agent through `SSH_AUTH_SOCK` (1Password or ssh-agent); Local Network permission prompt.
- **Hardware and outside services:** a serial device; a real 2FA/PAM server, if the Docker one isn't enough; the AI with a real API key (one suggestion); Mosh over a Wi-Fi drop.
- **Release:** a signed and notarized build opens without a warning; the updater installs v0.1.1 over v0.1.0; the DMG, MSI or EXE, and AppImage install and launch.

Everything else in the old `smoke-test.md` moves to L6, L7 or L9 and is deleted from the manual list once its test exists.

## 11. Test servers (T2)

Extend `scripts/test-servers.sh` and add `tests/servers/compose.yml`. One network; keys and passwords are generated into `.test-servers/` (git-ignored) on `up`.

| name | image or basis | purpose |
| --- | --- | --- |
| `a` | linuxserver/openssh-server | password `test/test`; bastion; `AllowTcpForwarding yes`, `GatewayPorts clientspecified`, `AcceptEnv LANG LC_* DALAN_*` |
| `b` | linuxserver/openssh-server | key-only user `keyuser`; `TrustedUserCAKeys` for certificate tests |
| `c` | linuxserver/openssh-server | reachable only from `a` (inner) |
| `kbd` | small Debian sshd image | `KbdInteractiveAuthentication yes` with a PAM module asking "Verification code:" (a fixed code from `.test-servers/otp`) |
| `web` | nginx:alpine | target for tunnels (`-L`) |
| `squid` | ubuntu/squid | HTTP CONNECT proxy |
| `socks` | serjs/go-socks5-proxy (or dante) | SOCKS5 proxy |
| `telnet` | busybox telnetd | Telnet host |
| `mosh` | debian + openssh + mosh-server | Mosh |
| `slow` | `a` behind `tc netem` 800 ms | slow connect and step log |
| `oldkex` | sshd with only `diffie-hellman-group14-sha1` | key exchange failed message |

**Commands:**

```bash
scripts/test-servers.sh up | down | status | restart <name> | rotate <name> [--type ecdsa] | otp
```

- `rotate` replaces the host key, for F12.
- Every server's fingerprint is printed and saved to `.test-servers/servers.json`. The Rust L3 tests and the real-app tests read this file.

## 12. CI (`.github/workflows/ci.yml`)

| job | runs on | steps |
| --- | --- | --- |
| `static` | ubuntu | lint, typecheck, sweep, contrast, capabilities rule test |
| `unit` | ubuntu | Vitest (L4, L5) |
| `rust` | ubuntu, macos, windows | `cargo clippy -D warnings`, `cargo test` (L1, L2), the bindings diff |
| `rust-servers` | ubuntu + compose | `cargo test -- --ignored` (L3) |
| `browser` | ubuntu, in the Playwright image | build, `test:e2e`, `test:visual`, `walk:ci`; upload `playwright-report`, `walk-report`, videos |
| `app` | ubuntu + compose | `test:app` (L9): nightly, on `v*` tags and by manual dispatch; upload the video and gallery |

- **Required for merging and tagging:** `static`, `unit`, `rust`, `browser`.
- **Required for tagging:** `rust-servers` and `app` too.
- **Flaky tests:** the CI retries are only a signal. A test that needed a retry is reported in the summary. A test that flakes twice in a week gets `@quarantine` (it runs, but doesn't block) and an issue.

## 13. Definition of done for features

- Add columns to `replica/features.csv`: `test` (test IDs, `;`-separated) and `checked` (date of the last hand check, or empty).
- `clone=yes` needs a `test` or a `checked` value. A small script, `scripts/features-check.mjs`, run in `static`, fails when a `yes` row has neither.
- The script also validates that the CSV has 8 columns on every row. The current file has 4 malformed rows.
- A new feature's PR adds tests at the layers §6 and §7 name for it.

## 14. Phases

| phase | scope | acceptance |
| --- | --- | --- |
| **T1** foundations | `@playwright/test` config and projects; move the 2 existing e2e files; `tests/support/serve.mjs`; the full typed fake backend (§4) with the `team` and `empty` scenarios and scripted servers; the contract test; test hooks (§5) stripped from production | `pnpm test:e2e` green; contract test green; typecheck fails if a command lacks a fake (demonstrate once, then revert) |
| **T2** servers | Extend the test-server script and compose (§11); the L3 Rust tests read `servers.json`; CI `rust-servers` job | `pnpm test:servers:rust` green locally and in CI |
| **T3** walkthrough | `tests/walk/` chapters 1–17, step helper, captions, outline, gallery, `run.mjs` options; `walk:ci` in CI | `pnpm walk` runs headed end to end, opens the gallery; `pnpm walk --headless` green |
| **T4** browser coverage | Every E cell in §6 and every state in §7; visual baselines dark and light, made in the Playwright image; axe on every state; keyboard-only chapter; hard-rule tests (§9.1); security tests (§9.2) | `test:e2e`, `test:visual` green; zero serious axe violations; the coverage table in `replica/testing/coverage.md` generated from test IDs shows every cell filled |
| **T5** real app | `tests/app/` Docker runner, WebdriverIO, `Driver` interface, the A column of §6, `walk:app` with video; CI `app` job | `pnpm test:app` and `pnpm walk:app` green on Mehrdad's Mac through Docker Desktop and in CI |
| **T6** manual list and features | `hand-checks.md` (§10); the `features.csv` columns plus the check script; delete smoke steps now automated | The script passes; every `yes` row has a test or a check date |
| **T7** perf, migration, long run | §9.4, §9.5, the nightly long run | Thresholds met; fixtures for v0001–v0005 |

When all of T1 to T7 are `[x]`, update `CLAUDE.md` "Commands" with the new scripts and "Working in phases" step 4. From then on, run `pnpm test:all` and, for UI phases, `pnpm walk --headless`.
