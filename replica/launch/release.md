# Releasing Dalan

Pushing a tag like `v0.1.0` builds installers for macOS (universal), Windows (x64) and Linux
(AppImage, deb, rpm), signs the update files, and puts everything on a **draft** GitHub Release with
`SHA256SUMS.txt` and `latest.json` (what the in-app updater reads). Publish the draft by hand when
you have tried the installers. Workflows: `.github/workflows/release.yml`, `pages.yml`.

## One-time setup

1. **Updater key pair** (on your Mac; keep the private key out of the repo):

   ```bash
   pnpm tauri signer generate -w ~/.tauri/dalan-updater.key
   ```

   - Put the **public** key (the `.key.pub` text) in `src-tauri/tauri.release.conf.json`, replacing
     `REPLACE_WITH_UPDATER_PUBLIC_KEY`, and commit it.
   - In GitHub -> Settings -> Secrets and variables -> Actions, add `TAURI_SIGNING_PRIVATE_KEY`
     (the private key file's contents) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (the password you chose).
   - **If you lose this key, installed apps can never update again.** Back it up in your password manager.
2. **GitHub Pages:** Settings -> Pages -> Source: *GitHub Actions*. The download page publishes on every
   push to `main` that touches `site/`, and again when a release is published (it then shows that version).
3. **macOS signing (optional until the Apple Developer account exists).** Without these secrets the build
   is unsigned and people open it with right-click -> Open. To sign and notarize, add:
   `APPLE_CERTIFICATE` (base64 of the Developer ID `.p12`), `APPLE_CERTIFICATE_PASSWORD`,
   `APPLE_SIGNING_IDENTITY` (like `Developer ID Application: Name (TEAMID)`), `APPLE_ID`,
   `APPLE_PASSWORD` (an app-specific password) and `APPLE_TEAM_ID`.
4. **Windows signing:** not set up. Installers show a SmartScreen warning until a certificate is added.

## Each release

1. Set the new version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json` (the same
   number), commit, and push to `main`.
2. `git tag v0.1.1 && git push origin v0.1.1`. The workflow refuses a tag that doesn't match `tauri.conf.json`.
3. Wait for the three builds and the checksums job, open the draft release, try an installer, then publish it.
4. Installed apps see the update under Settings -> About -> Check for updates.

## Good to know

- Dev builds have no updater (the config with `plugins.updater` is only added by the release build), so
  "Check for updates" says updates aren't set up there.
- The updater only installs files signed by the private key above, so a hijacked release can't push code.
- `OWNER/dalan-app` in `tauri.release.conf.json` and `site/index.html` is replaced by the workflows with the
  repository's real name.
