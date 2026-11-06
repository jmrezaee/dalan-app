# Release setup: what is left for you

How-to for each step is in `replica/launch/release.md`. Checked 2026-11-06.

## Already in place

- GitHub repository `jmrezaee/dalan-app`, with `origin` set.
- Updater key pair. The public key is in `src-tauri/tauri.release.conf.json`; the private key and its password are in `~/.tauri/`.
- GitHub secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
- Pages workflow ran green. CI ran green on `49c3fee` (including the `e2e` and `docker-ssh` jobs).
- A draft release `Dalan v0.1.0` exists on GitHub. It was built from tag `v0.1.0` at `49c3fee`, which has none of P21 to P25.

## Still yours

- [ ] **Back up the updater private key and its password** in your password manager. Lose them and installed apps can never update.
- [ ] **Decide what happens to the old `v0.1.0`.** The local tag is deleted. The tag and draft release still exist on GitHub. Before the first real release, delete that draft release and tag on GitHub (outward-facing, so Claude asks first), then tag again only after the new commits have a green CI run (P29).
- [ ] **Push the P21 onward commits** when you are ready. Nothing is pushed until you say so.

## Can wait

- [ ] **Apple Developer account**, for a signed and notarised Mac app. Secrets: `APPLE_CERTIFICATE` (base64 `.p12`), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific), `APPLE_TEAM_ID`. Without them the build is unsigned and people open it with right-click, Open.
- [ ] **Windows code signing** (optional). Without it Windows shows a SmartScreen warning.
- [ ] **A real screenshot** for the download page (`site/index.html` still shows a placeholder box).

## Then (P29)

1. Check the version is the same in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json`.
2. `git tag v0.1.0 && git push origin v0.1.0`. The workflow builds macOS, Windows and Linux and makes a **draft** release.
3. Try an installer from the draft, then publish it.
