# Launch plan: Dalan team rollout

The goal is for the whole team to use Dalan daily, instead of their current SSH setup, without anyone losing access to a server.
This is an internal rollout, not a public launch, so it has no Product Hunt or Hacker News post and no waitlist. If Dalan ever goes public, run `/replica-entrepreneur` first and rewrite this plan.

## Phases

| phase | when | who | exit criteria |
| --- | --- | --- | --- |
| 0. Dogfood | from M1 (vertical slice) | Mehrdad only | You use Dalan for your own daily SSH for a week, with your old client still installed as a fallback. |
| 1. Pilot | after M2 (must-haves) | 2–3 teammates who use SSH most | A week of daily use. No blocker bugs open. Every pilot user imported their servers without help. |
| 2. Team | after M3 (should-haves) | everyone who touches servers | 80% of the team use it weekly. The old client stays allowed but unsupported. |
| 3. Default | 1 month after phase 2 | new joiners | Onboarding docs say "install Dalan". Server details live in a shared CSV/SSH-config import pack, not in chat. |

## Before the pilot (checklist)

- [ ] **Releases:** `tauri-action` in GitHub Actions builds macOS (universal), Windows (x64) and Linux (AppImage/deb/rpm) on every `v*` tag and attaches them to a GitHub Release with checksums.
- [ ] **Updates:** the Tauri updater plugin is on and points at the GitHub Releases `latest.json`, with the updater key pair generated and the private key in CI secrets only. Without it, nobody upgrades.
- [ ] **macOS signing:** enrol in the Apple Developer Program (US$99/year; see `pricing.md`). Developer ID certificate and notarization run in CI. Skip this only while it's just you.
- [ ] **Windows signing:** optional for the pilot. Add Azure Artifact Signing before phase 2.
- [ ] **Download page:** `site/index.html`. Set `REPO` and `VERSION` in its script and publish with GitHub Pages. Replace the screenshot placeholder with a real screenshot.
- [ ] **Crash and error reporting:** opt-in and local-first, to keep the no-telemetry promise.
  - "Copy diagnostics" in Settings → About bundles the version, OS and the last 200 log lines, **with hostnames and usernames redacted**, ready to paste into an issue.
  - No automatic analytics. Sentry or similar only if the team agrees, opt-in, with hostnames and usernames scrubbed.
- [ ] **Feedback channel:** one place for problems, either GitHub Issues with a bug template (version, OS, steps, expected, actual) or a team chat channel. Pick one.
- [ ] **Rebrand sweep and contrast check** pass in CI:
  - `python3 replica/sweep.py . --config replica/brand.json`
  - `python3 replica/design/contrast.py replica/design/tokens.json`
- [ ] **Import pack:** turn the team's real server list into one `~/.ssh/config`-style file or CSV that imports cleanly (S25, F10).
  - Keep it in a private repo or vault. **Never put passwords or private keys in it**, only hosts, users, ports, groups and jump hosts.
  - Each person adds their own key in Dalan.

## Onboarding (5 minutes, written up as a one-page "Getting started")

1. Download from the page and open it. On macOS, before notarization: right-click → Open.
2. Choose a master password. Explain once that there is no recovery and it should go in your password manager.
3. Import the team pack (Hosts → New ▾ → Import) and your own `~/.ssh/config`.
4. Generate or import your key (Keychain), then use **Export to host** on one server to check it works.
5. Press ⌘K, type a server name, Enter. Trust the fingerprint the first time, after checking it against the team's list of known fingerprints.

## The first 10 users (by hand)

There are no strangers to recruit; this is the team. For each person:

- Sit with them (or screen-share) for their first import and first connection. Watch, don't help. Write down every hesitation, because those are the real bugs.
- Ask three questions after a week:
  - What did you still open your old client for?
  - What was slower than before?
  - What would you miss if Dalan disappeared?
- Log their answers in `replica/launch/feedback-log.md`, one line per finding, tagged with a screen ID (S02, S18…), and turn repeats into issues.

Rough order: you, the person who manages the most servers, the person with the most jump-host setups, a Windows user, a Linux user, a light user (support team member who only occasionally connects), then the rest.

## What to watch (no analytics, so ask and count)

- **Weekly:** who opened Dalan this week (ask in the team channel; a show of hands is fine).
- **Bugs:** open blocker bugs = 0 before each phase gate.
- **Fallbacks:** each "I had to use my old client because…" is a missing feature. Add it to `features.csv` notes and move its priority up.

## Risks and how we handle them

| risk | handling |
| --- | --- |
| Someone forgets the master password | Expected. They re-import the team pack and re-add their key. The getting-started page says this up front. |
| A bad build locks people out of servers | The old client stays installed through phase 2. The updater can roll back by publishing a newer fixed version fast. |
| Keys leak through the import pack or chat | The pack contains no secrets (checklist rule). Each person's private key never leaves their machine. |
| Host-key prompts get clicked through | Publish the team's server fingerprints next to the import pack. The "identity changed" screen blocks by default (S18, F12). |
| Server-side surprises (AcceptEnv, GatewayPorts, SFTP disabled) | The error copy names the server setting. Collect real cases in the feedback log. |

## Next step

`/replica-deploy`. For Dalan that means the release pipeline rather than a web host:

- signing and notarization
- updater keys
- the GitHub Pages download page
- the CI gates listed above
