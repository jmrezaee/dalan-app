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

## Silent install (Windows)

For IT and scripts. The setup takes Tauri's options and ours. Examples:

```
Dalan_0.1.2_x64-setup.exe /S /DESKTOP=1 /SSHLINKS=1
Dalan_0.1.2_x64-allusers-setup.exe /S
"%LOCALAPPDATA%\Dalan\uninstall.exe" /S /DELETEDATA=1
```

| option | meaning |
| --- | --- |
| `/S` | silent: no pages, the defaults below |
| `/STARTMENU=0\|1` | Start menu shortcut (default 1) |
| `/DESKTOP=0\|1` | desktop shortcut (default 0) |
| `/SSHLINKS=0\|1` | open `ssh://` links with Dalan (default 0) |
| `/AUTOSTART=0\|1` | start Dalan when signing in to Windows (default 0) |
| `/UPDATE` | what the auto-updater runs: keeps the saved choices, shortcuts and data |
| `/NS` | no shortcuts at all |
| `/D=<folder>` | install folder (must be last) |
| uninstaller `/DELETEDATA=1` | also delete the vault and settings (a plain `/S` keeps them) |

A choice that is not on the command line comes from the last install's saved choices, then from the default.
Only one edition can be installed on a PC: the second setup stops with a message (and a non-zero exit code).
