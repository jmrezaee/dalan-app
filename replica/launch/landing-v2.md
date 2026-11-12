# Download page v2: product-led dark

The redesign of `site/index.html`, the team's download page on GitHub Pages. Same audience and job as today (teammates get the right installer fast), but led by real product screenshots instead of text and a placeholder.

- **Design canvas:** https://claude.ai/artifact/VWhMrR9t799hwjzeKda32n (private to Mehrdad; open it with the Artifact read action, or ask him for screenshots). Three boards:
  - `Main.dc.html`: desktop at 1440. It has a Visitor tweak (`os`: mac / windows / linux) that switches the hero button and highlights the matching download card.
  - `Phone.dc.html`: the 390-wide layout.
  - `Download.dc.html`: the hero button for each OS, the phone/unknown case, and the Windows card with both editions.
- Match the layout and copy from the canvas. Colours come from tokens (see "Build rules"); the canvas uses inline hex only because it is a mockup.
- Phase **L1** in `replica/progress.md`.

## Sections, in order

| # | id | content | screenshot |
| --- | --- | --- | --- |
| 1 | nav | Sticky, blurred. Mark + "Dalan"; links Features, Security, Download, Help (hidden under 1000 px; phone gets a menu button that opens the same links); version pill `v{VERSION}`; small primary "Download". | – |
| 2 | `top` hero | Status chip "Built for our team · free · macOS, Windows, Linux". H1 "Your servers, / one keystroke away." (second line in accent). Sub: "Dalan keeps hosts, keys, tunnels and files in one calm desktop app. Everything is encrypted on your computer. No account, no cloud." Primary = OS button (below), secondary "Other platforms" → `#download`. Meta line per OS. Under it a big screenshot with a soft turquoise radial glow, the palette screenshot overlapping bottom-right and a "⌘K · type a few letters, press Enter" keycap card bottom-left (both overlaps hidden under 1000 px). | `S18-connected`, `S17-palette` |
| 3 | chips | "Everything you reach for, in one window" + pills: SSH, SFTP, Port forwarding, Jump hosts, Snippets, Workspaces, Broadcast, Session logs, SSH keys and certificates, Telnet, Serial, Mosh. | – |
| 4 | `features` | Three alternating rows (text 5 / image 7, then flipped). Eyebrows in mono: "01 · Connect", "02 · Organise", "03 · Many servers". Copy as on the canvas. Row 2 overlaps the host editor on the grid. | `S17-palette`; `S02-grid` + `S03-edit-jump-page`; `S13-run-on-hosts` |
| 5 | `security` | Bordered band with a faint large arch in the corner. "04 · Safe by default" / "Protective, and quiet about it." Four points with Lucide icons (Lock, ShieldCheck, KeyRound, CircleSlash): Encrypted on this computer; Every server checked; Keys stay out of sight; No account, no telemetry. | `S18-identity-changed` |
| 6 | more | "The basics, done carefully." 3×2 cards with a 200 px cropped image (`object-fit: cover`, per-card `object-position` from the canvas): Files side by side, Tunnels that come back, Drops handled, A side panel for the terminal, Jump hosts and proxies, Locks itself. | `S21-sftp`, `S09-tunnels`, `S18-dropped-card`, `S20-side-panel`, `S03-edit-jump-page`, `S24-unlock` |
| 7 | `download` | "Get Dalan", "Free for the team. Version {VERSION} · Release notes". Three cards (macOS, Windows, Linux); the visitor's card gets the accent ring. Windows card: primary "Download installer" (just-me) and the link "Installing for everyone on a shared PC?" → all-users file. Linux: three secondary buttons. Below: "Checksums and older versions are on the releases page." | – |
| 8 | `help` | "Questions" left, `<details>` list right; first one open. Questions from the canvas (storage, forgotten master password, import, which Windows installer, sync, nothing on servers, macOS can't verify). | – |
| 9 | final CTA | Large mark, "Stop hunting for server details.", the OS button again. | – |
| 10 | footer | "Dalan · built in-house", Release notes, Report a problem, Licences; fonts and icon credits. | – |

Phone (under 640 px): one column, 16 px gutters, H1 40/44, screenshots full width with no overlaps, cards stacked, Linux buttons stay in a row of three. The hero button reads "See downloads" on phones (Dalan is desktop only) with the note "Dalan is a desktop app. Open this page on your computer, or send yourself the link."

## The download button

Keep the existing script's `REPO`, `VERSION` and asset names (the Pages workflow fills `REPO` and `VERSION`; don't change the `const REPO = '…'` / `const VERSION = '…'` lines it seds):

| visitor | hero label | href | meta line |
| --- | --- | --- | --- |
| macOS | Download for macOS | `Dalan_${VERSION}_universal.dmg` | Apple silicon and Intel · macOS {min} or later |
| Windows | Download for Windows | `Dalan_${VERSION}_x64-setup.exe` | Windows 10 and 11 · no administrator rights needed |
| Linux | Download for Linux | the `.AppImage` asset (check the real file name on the v0.1.2 release) | AppImage, .deb and .rpm |
| phone, tablet or unknown | See downloads | `#download` | – |

- Detect with `navigator.userAgentData?.platform`, falling back to `navigator.userAgent`. iPhone, iPad, Android → the phone case.
- All-users link: `Dalan_${VERSION}_x64-allusers-setup.exe`. It is a link, never a second primary button.
- Linux buttons point at the real `.AppImage`, `.deb`, `.rpm` names from the release; if a name is unknown, link to `/releases/latest`.
- Without JavaScript the page still works: the static HTML shows "See downloads" and every card links to `/releases/latest`.
- `{min}`: use the real macOS minimum. `tauri.conf.json` sets no `bundle.macOS.minimumSystemVersion`, so either set one (ask Mehrdad) or drop the "or later" clause. The canvas's "macOS 12" is a placeholder.
- The canvas shows `v0.1.1`; the page always shows `VERSION`.

## Screenshots

- **Source:** the visual-test baselines in `tests/visual/__screenshots__/` (fake backend, made-up hosts, 1280×800, dark). They are deterministic and contain no real hostnames or keys.
- **Script:** add `pnpm site:shots` (`scripts/site-shots.mjs`). It copies the 11 files in the table above into `site/img/`, converts them to WebP (quality ~85) plus a 640-wide version for phones, and writes `site/img/shots.json` with width and height. Run it after `pnpm test:visual:update` when the UI changes.
- In the page: `<img>` with `srcset` (640w and 1280w), `width`/`height` set, `loading="lazy"` except the hero, `decoding="async"`, and the alt text from the canvas.
- Keep the whole `site/` under ~2 MB.
- If a baseline ever shows a placeholder or debug state, retake it from `pnpm walk:app` instead.

## Build rules

- Still one static page, pre-built with the tokens, no framework at runtime. Tailwind classes from tokens only (`bg-bg`, `bg-surface`, `border-border`, `text-muted`, `bg-accent`, `text-on-accent`, …). Add tokens for anything new (the glow, the deeper page background `chrome`, the card background) to `tokens.json` rather than hex in the page, then rebuild and run `contrast.py`.
- Dark only (the page is `class="dark"`); no theme switch.
- Fonts: Inter and JetBrains Mono, served from `site/fonts/` (copy the app's bundled files). No Google Fonts request.
- Icons: inline Lucide SVGs (ISC). For the OS cards use Lucide `Laptop` (macOS), `Monitor` (Windows) and `Terminal` (Linux) rather than the vendors' logos the canvas sketches.
- Accessibility: one `h1`; skip link; visible focus ring (`:focus-visible` accent outline); every screenshot has alt text; FAQ uses `<details>`/`<summary>`; contrast AA (the muted greys on the canvas such as `#7a8494` must map to a token that passes on `chrome`; check with `contrast.py`).
- `prefers-reduced-motion`: no smooth scroll, no transitions.
- No claims we can't back: no download counts, no testimonials, no "signed" or "notarised" until P29 adds signing, no "sync" beyond "not yet".
- CSP-friendly: no inline event handlers; the one script stays small and at the end of `body`.
- Brand sweep must stay clean.

## Checks for L1

- `python3 replica/sweep.py . --config replica/brand.json`, `python3 replica/design/contrast.py replica/design/tokens.json`.
- Playwright: `tests/site/site.spec.ts` (new) opens `site/index.html` from disk at 1440 and 390 with each OS user agent and checks the hero label and href, the all-users link, `#download` anchors, no horizontal scroll at 390, every `img` has alt and loads, and axe has no serious violations. Save `site-1440.png` and `site-390.png` and compare them with the canvas by eye.
- Lighthouse (optional): performance and accessibility at least 95 on the phone profile.
