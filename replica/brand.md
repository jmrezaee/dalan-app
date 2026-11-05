# Brand: Dalan (دالان)

Date: 2026-11-04. Angle: there is no `replica/fixes.md` yet, because `/replica-entrepreneur` hasn't run. The working angle is **a calm, private, local-first SSH client for you and your team. Your servers, your keys, nothing in someone else's cloud.**

## Name

**Dalan** (Persian دالان: a covered passage or corridor, the way through a house to its inner rooms; in Cebuano, spoken in the Philippines, *dalan* means "road").

- It fits the product: a passage to your servers, and SSH tunnels and jump hosts are literally passages.
- Five letters, said the way it's spelled, works in English and Persian.
- No negative meaning found in English, Persian, Arabic, Turkish, Spanish, French or German. Ask native speakers before launch, because this is a screening, not a guarantee.
- Not similar to the original in sound, look or meaning. It shares no "Term-" stem; Terminus, Termix, Termora and Termux crowd that space.

### How the shortlist was made

20 first candidates, plus 5 replacements after screening:

| style | candidates | cut because |
| --- | --- | --- |
| descriptive | Hostdeck, Shellroom, Tunnelbox, Serverdesk, Keyhop | **Hostdeck**: an SSH+SFTP macOS app already uses it (github.com/byurself/HostDeck). Shellroom, Tunnelbox, Serverdesk: generic. |
| compound | Hopwise, Portwell, Shellwise, Keyline, Hopline | "-wise" is overused. Keyline and Portwell are weak. |
| invented | Konsu, Sorvo, Velo, Ziro | **Konsu** is too close to KDE's *Konsole* and HashiCorp's *Consul*. Velo and Ziro are taken in tech. |
| metaphor | Pier, Berth, Harbor, Lantern, Tether, Causeway, Gangway, Mooring, Dalan | **Pier**: a terminal workbench and "Pier-X" already exist. **Berth**: a Mac SSH client exists (github.com/xinghelee/Berth). **Lantern**: a VPN. **Tether**: a stablecoin. **Harbor**: the CNCF registry. **Causeway**: Apache Causeway. **Gangway**: a Kubernetes auth tool (vmware/gangway). |
| verb | Hop, Reach, Dock | Generic. Dock clashes with Docker and the macOS Dock. |

Final 5, with Dalan chosen by you: **Dalan**, Keyhop, Mooring, Hopline, Sorvo.

### Screening checks for Dalan

These are screening checks, not legal clearance. Before you spend money on the name (domain, store listing, logo), have a trademark lawyer run a proper search.

| check | where | result |
| --- | --- | --- |
| the web | search "Dalan" + SSH / terminal / developer tool | 2026-11-04: no SSH, terminal or dev-tool product found. |
| GitHub | `gh search repos dalan` | 2026-11-04: only small unrelated projects (a game, "dalang" video editor). No SSH client. |
| App Store / Play | search the exact name "Dalan" | **to run**. My web search found no app called Dalan in the SSH category, but search the stores directly. |
| US trademark | https://tmsearch.uspto.gov, classes 9 and 42 | **to run** |
| Canada | https://ised-isde.canada.ca/cipo/trademark-search/srch, classes 9 and 42 | **to run**. This matters most, because you're in Canada. |
| EU | https://euipo.europa.eu/eSearch or https://www.tmdn.org/tmview | **to run** |
| global | https://branddb.wipo.int | **to run** |
| domain | dalan.app, dalan.dev, getdalan.com, dalan.sh | **to run**. WHOIS/RDAP was blocked from this session; check at your registrar. Expect dalan.com to be taken (short word). |
| handles | GitHub org `dalan-app`, X / Instagram `@dalanapp` | **to run** |

If Dalan fails a trademark check in classes 9 or 42, the next pick is **Keyhop**, then **Mooring**.

## Palette

The original's mark is plain black and white (source: Simple Icons via dashboardicons.com). Ours is **turquoise**, the colour of Persian tilework (*firouzeh*). It is a different family from the original, and from the placeholder blue it replaces.

| role | dark (default) | light | contrast |
| --- | --- | --- | --- |
| accent | `#3cc8c0` | `#0b7a76` | 9.19 on bg in dark, 5.17 in light |
| accent-hover | `#66d6cf` | `#086561` | |
| on-accent | `#04201e` | `#ffffff` | 8.30 / 5.17 |
| success (moved to yellow-green so it doesn't read as the brand) | `#86d05c` | `#3a7a1c` | AA on every surface |
| terminal cursor | accent | accent | |

The neutrals, danger, warning and terminal ANSI colours are unchanged from `/replica-design`. Re-checked with `python3 replica/design/contrast.py replica/design/tokens.json`: 94 pairs, **0 AA failures**. See `replica/design/preview.png`.

Turquoise and the success green are close-ish for people with colour-blindness. That's why status always comes with an icon and a label, never colour alone (rule in `components.md`).

## Logo brief

- **Idea:** a passage you can see through. A pointed Persian arch (an *iwan*) seen head-on, with a lit opening at the far end. The opening is a small block that reads as a terminal cursor. The way through to your server.
- **Mark type:** symbol + wordmark. The symbol works alone as the app icon.
- **Symbol:** the arch outline in turquoise `#3cc8c0` on a deep neutral (`#0f1115`), with the cursor block in a lighter turquoise `#66d6cf`. A one-colour version (white on turquoise, turquoise on white) must also work.
- **Wordmark:** "Dalan" in a geometric sans with open shapes, set in Inter Display / Inter Tight semibold or a custom drawing. Lowercase "dalan" is fine too. An optional Persian lockup is «دالان» in Vazirmatn (OFL).
- **Sizes:** it must read at 16px (favicon, tab icon) and at 1024px (app icon). At 16px it reduces to the arch plus the cursor, with no inner detail.
- **Deliverables:**
  - master SVG (symbol, wordmark, horizontal lockup)
  - macOS `.icns` from a 1024×1024 PNG on the macOS icon grid (rounded square, no transparency in the 1024 source)
  - Windows `.ico` (16, 24, 32, 48, 64, 256)
  - Linux PNGs (32–512)
  - `tauri icon` input: a 1024×1024 PNG
  - social / README image, 1200×630
- **Must not resemble the original's mark.**
  - Put the drafts next to the original's logo (termius.com/brand-resources) and check: no shared shape, no shared letterform trick, no black/white-only version as the primary.
  - Also avoid the `>_` prompt glyph that most terminal apps use. The cursor block inside an arch is ours, a prompt chevron is not.
- **Making it:** design it yourself in Figma, brief a designer with this section, or generate drafts with an image model and redraw the chosen one as a clean vector. Don't ship a raw generated bitmap as the master.

## Voice

**Calm, precise, protective.**

| word | means | does not mean |
| --- | --- | --- |
| calm | Steady when things break. No exclamation marks, no blame. | sleepy, vague, or hiding that something is wrong |
| precise | Names the exact host, port, file or key, and says what happened. | jargon dumps or raw error codes as the message |
| protective | Defaults to safe and explains risk in one plain sentence. | paranoid, preachy, or nagging about things you chose |

| do | don't |
| --- | --- |
| "Can't reach 10.0.0.5:22." | "Oops! Something went wrong 😕" |
| "Delete 3 hosts" | "OK" / "Yes" |
| "This server's identity changed. Don't continue unless you expected it." | "WARNING!!! POSSIBLE MITM ATTACK!!!" |
| "Saved." | "Your host has been successfully saved!" |
| "Sign-in failed: the server rejected this key." | "Error: Auth fail (code 14)" |

## The 10 most-seen strings, in this voice

All are written fresh. These are the source of truth; `replica/design/components.md` uses the same wording.

| # | where | string |
| --- | --- | --- |
| 1 | S24 first run | **Set up Dalan** · "Choose a master password. It encrypts your keys and passwords on this computer. There's no way to recover it, so keep it somewhere safe." · [Create vault] |
| 2 | S24 unlock | **Unlock Dalan** · [Unlock] · wrong: "That password didn't work." |
| 3 | S02 empty | **No servers yet** · "Add one by hand, or bring them over from your SSH config." · [Add host] [Import] |
| 4 | S02 main button | **New host** |
| 5 | S18 first connect | **First time connecting to {host}** · "Check that this fingerprint matches the server:" · [Cancel] [Trust and connect] |
| 6 | S18 key changed | **This server's identity changed** · "That can happen after a reinstall, or it can mean someone is intercepting the connection. Don't continue unless you expected it." · [Close] · "Replace saved key…" |
| 7 | S18 auth failed | **Sign-in failed** · "{host} rejected {the password / the key "deploy-ed25519"}." · [Edit host] [Try again] |
| 8 | S18 unreachable | **Can't reach {address}:{port}** · "Check the address and port, your VPN, or a firewall in between." · [Retry] |
| 9 | S18 disconnected | **Connection closed** · "{host} disconnected at {time}." · [Reconnect] [Close tab] |
| 10 | S09 port in use | "Port {port} is already in use on this computer. Pick another, or stop whatever is using it." |

Toast pattern: "Saved." · "Public key copied." · "Tunnel {label} is running." · "Copied 12 files to /var/www."

## Sweep

`replica/brand.json` lists the original's name and former name (`Termius`, `Server Auditor`) and its domains. Run it from the project root:

```bash
python3 replica/sweep.py . --config replica/brand.json
```

The sweep skips `replica/`, which is where the original's name belongs. Status on 2026-11-04: clean (see the run in this session). The codebase doesn't exist yet, so run the sweep again in CI on every build from M0 on. By eye, also check the app icon, window title, About box, installer name, bundle id (`app.dalan.desktop`) and the README.

**Naming in code:** the product name is **Dalan**. The package, crate, binary and repo name is **`dalan-app`**, and the bundle id is `app.dalan.desktop`. The local folder keeps its name, `termius-app`, by choice. The sweep never reads the root folder's own name, but nothing inside it may use that name.
