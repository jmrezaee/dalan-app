# Dalan (دالان)

A personal desktop SSH client for Mehrdad and his team. It is a clean-room rebuild of an existing SSH client's features and flows; the original is named only in `replica/`. It does not copy the original's code, assets, copy or brand.

- **Name:** Dalan is the product name, shown in the UI, window title and installers.
- **Code name:** `dalan-app` is the npm package name (`package.json`), the Cargo package (`src-tauri/Cargo.toml`, Rust lib `dalan_app_lib`), the binary and the GitHub repo.
- **Bundle id:** `app.dalan.desktop`. In `tauri.conf.json`: `productName: "Dalan"`, `identifier: "app.dalan.desktop"`.
- **The local project folder keeps its current name.** Don't rename it, and never put the folder's name in code, package metadata, the README or the UI.
- **Brand:** see `replica/brand.md` for the turquoise palette, logo brief, voice and the 10 key UI strings.

## Read these first

- `replica/recon.md`: screens **S01–S25**, flows **F01–F12**, components, inferred data model. Refer to screens and flows by ID in code comments, commits and PRs.
- `replica/features.csv`: feature matrix. When a feature works end to end, set `clone` to `yes`. Leave `skip` rows alone.
- `replica/architecture.md`: stack, Tauri command list, session/tunnel/transfer event types, background tasks, "parts that bite", build order **M0–M5**.
- `replica/design/`: design system.
  - `tokens.json` is the source of truth.
  - `tokens.css` is generated, and so is `terminal-themes.ts` (xterm themes).
  - `components.md` holds a spec for every component, with states, keyboard and a11y behaviour, and our own copy.
  - `contrast.py` must report 0 AA failures.
  - `preview.html` / `preview.png` are a visual reference.
- **UI/UX design canvas:** https://claude.ai/artifact/U8iEwGC82GSXeTKYXdzanJ (private to Mehrdad; open it with the Artifact read action, or ask him for screenshots). It holds 23 artboards covering S01–S25, every connection state, components, foundations and the light theme. Match layouts and copy from it, using tokens, never its inline hex.
- `replica/schema.sql`: SQLite migration `0001_init` (21 tables). It is the source of truth for the DB. Change the schema only by adding a new migration, never by editing 0001 once it has shipped.

## Stack (do not swap without asking)

- **UI:** Nuxt 4 with `ssr: false` (SPA served by Tauri), TypeScript strict, Pinia, shadcn-vue (Reka UI), Tailwind, `@xterm/xterm` + addons
- **Desktop:** Tauri 2. **All SSH, SFTP, tunnels, PTY, filesystem, crypto and DB live in Rust** (`src-tauri/`).
- **Rust crates:** `russh`, `russh-sftp`, `ssh-key`, `portable-pty`, `rusqlite` (bundled) + `rusqlite_migration`, `argon2`, `chacha20poly1305`, `zeroize`, `notify`, `uuid` (v7), `thiserror`, `serde`, `tokio`
- **Database:** SQLite, one file in the app data dir, opened only by Rust. `PRAGMA foreign_keys=ON; journal_mode=WAL`.
- **Package manager:** **pnpm** only. Node **24.12.0**: run `nvm use 24.12.0` before any node/pnpm command.
- **Dev server port:** 4000 (set `devUrl` in `tauri.conf.json` to match)

## Hard rules

1. **Secrets never reach JavaScript.**
   - Passwords, passphrases and private keys go UI → Rust once, then live only encrypted in `secrets`, or decrypted in Rust memory under `zeroize`.
   - Commands return flags like `has_password`, never the secret itself.
2. **The webview never touches the network or the filesystem.** Do not enable the Tauri `fs`, `http` or `shell` plugins for the webview. Add a Rust command instead. Keep `capabilities/default.json` minimal.
3. **Terminal data is bytes.**
   - Stream output to xterm.js as raw bytes over a Tauri `Channel`, batched (~8 ms / 64 KB), with backpressure.
   - Never convert PTY output to `String` in Rust.
4. **Terminal tabs are not routes.** Keep live sessions mounted (`v-show`, keep-alive layer). An xterm instance must not unmount while its session is alive.
5. **Host keys are always verified.**
   - Unknown key: prompt, trust on first use.
   - Changed key: block. Default is cancel. Replacing it needs an explicit confirmation.
   - No "accept all" setting.
6. **Types are generated, not hand-written.** Generate the TS types for commands and events from Rust (tauri-specta or ts-rs) into `app/types/bindings.ts`.
7. **Errors are one shape.** `AppError` serialises to `{ code, message }`. Use the codes listed in `architecture.md`. The UI maps codes to friendly copy.
8. **Clean room.**
   - Do not fetch, read or copy the original app's bundle, private APIs, icons, illustrations, theme names or UI copy.
   - Outside `replica/`, never write the original's name or domain anywhere: code, comments, commits, the README or the UI.
   - `python3 replica/sweep.py . --config replica/brand.json` must pass (exit 0). Run it in CI.
   - Public docs are fine for behaviour only.
   - Hands-on use of your own copy of the original is allowed to observe behaviour only: no copied text, nothing from Mehrdad's machine imported, and notes go in `replica/` in our own words.
   - Use our own name and brand (see `/replica-brand` later).

## Conventions

- **Nuxt:** `srcDir` is `app/`. The app is a single-page shell: vault sections, `sftp` and `settings` switch on a ref in `pages/index.vue`, because terminal tabs must stay mounted. Components are grouped by area (`hosts/`, `keychain/`, `terminal/`, `sftp/`, `workspace/`, `shell/`). shadcn-vue components go in `components/ui/`.
- **State:** one Pinia store per area. Stores call typed wrappers in `composables/useTauri.ts`. Components reach Rust through stores, not through `useTauri()` or `invoke` directly (P27 moved the last ones).
- **Rust:** `commands/` holds thin `#[tauri::command]` functions. The logic lives in modules (`db`, `vault`, `ssh`, `sftp`, `tunnels`, `keys`, `pty`, `import`, `logs`). DB access goes through `spawn_blocking` and one `Mutex<Connection>`.
- **Mehrdad is a TypeScript/Nuxt developer** who is growing his Rust. In Rust code, add short comments explaining ownership, async and lifetimes where they are not obvious. Prefer clear code over clever code.
- **Inheritance:** NULL in `connection_profiles` means inherit. Resolve host → group → parent group → app defaults per field, and return which group each value came from (for "from group X" hints in S03).
- **Time:** store UTC epoch milliseconds and display in local time.

## Design rules

- **No raw hex or ad-hoc colours in components.** Use token utilities only (`bg-surface`, `text-muted`, `border-border-input`, `bg-accent`, `bg-primary`, …).
- **Changing a token:**
  1. Edit `replica/design/tokens.json`.
  2. From `replica/design/`, run `python3 build_css.py`.
  3. Run `python3 contrast.py tokens.json`.
- **Naming clash:** shadcn-vue's `accent` means a hover background, but ours is the brand colour. When adding a shadcn-vue component, change its hover classes to `bg-surface-hover`.
- **Theme and assets:**
  - Dark is the default theme (`@nuxtjs/color-mode`, `preference: 'dark'`, `classSuffix: ''`).
  - Icons come from Lucide only, via `@lucide/vue` (the successor of the deprecated `lucide-vue-next`; shadcn-vue generates imports from it).
  - Fonts are Inter and JetBrains Mono, bundled locally.
- **Specs:** every component follows `replica/design/components.md`, including its focus, disabled, loading, error and empty states. UI copy comes from there or is written fresh in the same voice.
- **M0 builds `app/pages/design.vue` first** (dev only). It renders every primitive in every state in both themes, before any real screen.

## Commands

```bash
nvm use 24.12.0
pnpm install
pnpm tauri dev            # app with hot reload (UI on :4000)
pnpm tauri build          # release bundle
pnpm lint && pnpm typecheck
pnpm test                 # Vitest (UI)
cd src-tauri && cargo clippy -- -D warnings && cargo test
```

## Where we are

- Recon, architecture, the design system and the brand (Dalan, turquoise) are done.
- Launch is a team rollout, not public. See `replica/launch/launch-plan.md` (phases, checklist, onboarding).
- The download page is `site/index.html`. It is static, pre-built with the tokens. Set `REPO`/`VERSION` in its script.
- **P0 to P14 are built.** The 2026-11-05 audit (`replica/audit-2026-11-05.md`) found that most of it has never run in the real window, and listed security, robustness and UI fixes.
- **P15 to P25 are built.** The second audit (`replica/audit-2026-11-06.md`) lists what's left.
- **Testing:** the test strategy is `replica/testing/test-plan.md`: layers, fake backend, coverage matrix, `pnpm walk` and `pnpm walk:app`.
- **P26 is done.** `origin` is `jmrezaee/dalan-app` on GitHub; CI is green on the old `v0.1.0` commit; the P21 onward commits are not pushed.
- **P27 is built** (`[h]`, waiting for the hand-check session).
- **T1 is done.** **Next:** T2 to T7, then P28 (hand checks) and P29 (first release), in `replica/progress.md`. Type `go` to run the next one.
- **Manual checks:** a phase that needs Mehrdad to test something by hand is not done until he has reported back.
- **M1 is done when F01, F11 and F12 work against a real server:**
  - unlock
  - add a host with a password
  - connect
  - trust the key
  - use the shell
  - drop and reconnect
  - see the "key changed" block

## Working in phases: the `go` command

When Mehrdad types `go` (alone, or as "go" plus a note):

1. **Pick the phase.**
   - Read `replica/progress.md` and take the first phase marked `[ ]` or `[~]`. Skip `[x]` (done) and `[h]` (built, waiting for the hand-check session in P28).
   - Re-read the parts of `replica/architecture.md`, `replica/design/components.md` and the design canvas that the phase touches.
2. **Plan.** Post a short plan:
   - the screens, flows and commands to build
   - the files to touch
   - any decision that needs Mehrdad
   If a decision changes the stack, the schema or a hard rule, ask and wait. Otherwise continue without waiting.
3. **Build** the phase completely, following every rule in this file.
4. **Check.** Run all of these and fix what fails:
   - `pnpm lint && pnpm typecheck && pnpm test`
   - `cargo clippy -- -D warnings && cargo test`
   - `python3 replica/sweep.py . --config replica/brand.json`
   - `python3 replica/design/contrast.py replica/design/tokens.json`

   For UI phases, also run the app and take a Playwright screenshot to compare with the canvas.
5. **Record.**
   - Mark the phase `[x]` in `replica/progress.md` and add its one-line note.
   - Set `clone=yes` in `replica/features.csv` for the features that now work end to end.
6. **Commit** locally with a message like `P4: terminal slice (S18, F01, F11, F12)`. Never push unless asked.
7. **Stop and report:**
   - what was built
   - exactly what Mehrdad should test by hand, and how (for example, the Docker SSH server from "Testing against real servers")
   - known gaps
   - the next phase's name

   Then wait for the next `go`.

If a phase is too big for one session, finish a coherent part and mark it `[~]` with a note saying where to resume. The next `go` continues it.

## Testing against real servers

Use a throwaway container for SSH tests, never a production box. For example, `docker run -d -p 2222:2222 -e PASSWORD_ACCESS=true -e USER_PASSWORD=test -e USER_NAME=test lscr.io/linuxserver/openssh-server`. Add a second container to test jump hosts. Do not commit real hostnames, keys or passwords.
