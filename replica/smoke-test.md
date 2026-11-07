# Dalan smoke test (real window)

This list used to hold about 60 minutes of clicking. Almost all of it is now run by machines (`replica/testing/coverage.md` shows which flow is covered by which test):

| Used to be by hand | Now |
| --- | --- |
| First run, unlock, wrong password (A1) | `pnpm walk` chapter 1, `pnpm test:app` (`smoke`) |
| Connect, trust the key, shell, resize (A2) | `pnpm test:app` (`smoke`, `f01-wrong-password`), `pnpm walk:app` |
| Drop and Reconnect (A3) | `pnpm test:app` (`f11-drop`), `pnpm walk` chapter 10 |
| Key changed, blocked, Replace (A4) | `pnpm test:app` (`f12-rotated-key`), `pnpm walk` chapter 11 |
| Connection states, keys, quick connect, groups, jump hosts (B) | `pnpm test:e2e`, `pnpm walk`, `pnpm test:app` (`f02-key-login`, `f05-jump-host`) |
| Tunnels, snippets, import (C) | `pnpm test:e2e`, `pnpm test:app` (`f06-tunnel`), `pnpm walk` |
| Colours, spacing, both themes, accessibility (D) | `pnpm test:visual`, `pnpm test:a11y` |

Run the machines before you touch the app: `scripts/test-servers.sh up`, then `pnpm test:all` (see `replica/testing/test-plan.md`).

## What is still yours

Only what a Linux browser and the Linux build of the app cannot show: the real macOS window, system integration, hardware and outside services, plus the open findings the tests could not settle. That list is **`replica/testing/hand-checks.md`**. Run it before each release (P28, P29).

## Setup, if you need a server by hand

```bash
nvm use 24.12.0
scripts/test-servers.sh up        # prints ports, users and passwords
scripts/test-servers.sh status    # host-key fingerprints
pnpm tauri dev                    # the Dalan window; UI on :4000
scripts/test-servers.sh down      # when you are done
```

If port 4000 is taken, run it on 4001 without changing any file:

```bash
pnpm tauri dev --config '{"build":{"devUrl":"http://127.0.0.1:4001","beforeDevCommand":"pnpm dev --port 4001"}}'
```

To test a first run on a Mac that already has a vault, move the data folder aside (nothing is deleted) and put it back afterwards:

```bash
mv ~/Library/Application\ Support/app.dalan.desktop ~/Library/Application\ Support/app.dalan.desktop.bak
```

## Report back

Paste the filled-in lines of `hand-checks.md` (date, result, initials) to Claude. Don't fix anything while checking; just note it.
