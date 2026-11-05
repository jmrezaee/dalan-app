# Release setup: what only you can create

A checklist. The how-to for each step is in `replica/launch/release.md`; nothing here is done yet.
The first draft release can be cut once the first four boxes are ticked (the Apple and Windows items can wait).

## Needed for the first release

- [ ] **GitHub repository `dalan-app`.** Pick the owner (your account or an organisation) and create it, empty.
      Then, in this folder: `git remote add origin git@github.com:<owner>/dalan-app.git`. Nothing is pushed until you say so.
      The workflows replace `OWNER/dalan-app` (in `tauri.release.conf.json` and `site/index.html`) with the real name by themselves.
- [ ] **Updater key pair.** `pnpm tauri signer generate -w ~/.tauri/dalan-updater.key`.
      Put the public key in `src-tauri/tauri.release.conf.json` (replacing `REPLACE_WITH_UPDATER_PUBLIC_KEY`) and commit it.
      Back the private key up in your password manager: lose it and installed apps can never update again.
- [ ] **GitHub secrets** (Settings, Secrets and variables, Actions): `TAURI_SIGNING_PRIVATE_KEY` (the private key file's text)
      and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. The release workflow refuses to build without them.
- [ ] **GitHub Pages:** Settings, Pages, Source: *GitHub Actions*, so the download page (`site/`) publishes.
- [ ] **Make sure CI is green once.** The `e2e` and `docker-ssh` jobs in `.github/workflows/ci.yml` have never run on GitHub.

## Can wait

- [ ] **Apple Developer account**, for a signed and notarised Mac app. Secrets: `APPLE_CERTIFICATE` (base64 `.p12`),
      `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific), `APPLE_TEAM_ID`.
      Without them the build is unsigned and people open it with right-click, Open.
- [ ] **Windows code signing** (optional). Without it Windows shows a SmartScreen warning.
- [ ] **A real screenshot** for the download page (`site/index.html` still shows a placeholder box).

## Then

1. Check the version is the same in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json`.
2. `git tag v0.1.0 && git push origin v0.1.0`. The workflow builds macOS, Windows and Linux and makes a **draft** release.
3. Try an installer from the draft, then publish it.
