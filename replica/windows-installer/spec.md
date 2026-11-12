# Dalan Windows installer: design spec

Status: design, 2026-11-11. To be built in phases W1 to W4 (`replica/progress.md`).

Mockups: `replica/windows-installer/installer-mockups.png` (source: `mockups.html`).

Artwork, ready to use:

- `src-tauri/windows/installer-sidebar.bmp` (164×314)
- `src-tauri/windows/installer-header.bmp` (150×57)
- sources in `replica/windows-installer/art/`

## 1. Decisions (from Mehrdad, 2026-11-11)

| topic | decision |
| --- | --- |
| Engine | Tauri's **NSIS** bundler, branded. No WiX/MSI: CI already builds NSIS only, because the WiX download kept failing. |
| Who it installs for | **Two editions** from the same template. Tauri's `installMode: "both"` asks for administrator rights even when the person picks "Just me", so we don't use it. |
| Extras | Start menu and desktop shortcuts, launch on finish, `ssh://` link handler (opt-in), start with Windows (opt-in), keep or delete data on uninstall. |
| Language | English only. The copy is ours, in a custom language file. |
| Not in scope | A fully custom UI, a bootstrapper app, MSI, Persian, Windows Store. |

## 2. Editions

| | **Dalan (just me)**: the default download | **Dalan for all users** |
| --- | --- | --- |
| File | `Dalan_<ver>_x64-setup.exe` | `Dalan_<ver>_x64-allusers-setup.exe` |
| `installMode` | `currentUser` | `perMachine` |
| Admin rights | never | at start (UAC) |
| Folder | `%LOCALAPPDATA%\Dalan` | `%ProgramFiles%\Dalan` |
| Registry hive (uninstall key, choices) | HKCU | HKLM, plus HKCU for per-user extras (§5) |
| Auto-update | silent, no prompt | UAC prompt on each update |
| Updater endpoint | `.../latest.json` | `.../latest-allusers.json` (§8) |
| Who it's for | everyone by default | IT, shared or locked-down PCs |

- **Data is per user in both editions:** `%APPDATA%\app.dalan.desktop` (vault, logs, settings), plus WebView2 data in `%LOCALAPPDATA%\app.dalan.desktop`. Each Windows user has their own vault.
- **Mixed installs:** if the other edition is already installed, the installer says so and stops: *"Dalan is already installed for all users on this PC. Use that copy, or uninstall it first."* (and the reverse). Two copies would fight over the `ssh://` handler and autostart.

## 3. Page flow

**Installer (both editions):**

1. **Welcome:** sidebar artwork. The just-me edition explains that no admin rights are needed and points to the all-users installer.
2. **Already installed:** Tauri's default reinstall page, only when a previous version is found, with our copy:
   - "Update to X (recommended)", which keeps everything.
   - "Remove Dalan first, then install", which keeps data unless the person chooses otherwise in the uninstaller.
   - A newer version is already installed: "A newer Dalan (X) is already installed. Setup won't replace it." Downgrades stay off (`allow_downgrades` false).
3. **Folder:** Tauri's default page.
4. **Choose extras:** a **new** custom page (§4).
5. **Installing:** default page, with our log lines.
6. **Finish:** sidebar artwork, "Open Dalan now" (checked) and the master password hint. The desktop shortcut checkbox Tauri puts here **moves to page 4**, so all choices sit in one place.

**Uninstaller:**

1. **Your data:** a **new** custom page (§6) that replaces Tauri's "Delete app data" checkbox.
2. **Uninstalling:** default page.
3. **Done:** the text says where the data was kept, or that it was deleted.

There is no license page: Dalan is an internal team tool. If a license is added later, use `bundle.license`.

## 4. Page 4: Choose extras

Build it with nsDialogs inside the custom template (§7). Layout and copy follow mockup page 4.

| control | default (interactive) | silent default | effect |
| --- | --- | --- | --- |
| Add a Start menu shortcut | ✓ | ✓ | `$SMPROGRAMS\Dalan.lnk` (all users: common Start menu) |
| Add a desktop shortcut | ☐ | ☐ | `$DESKTOP\Dalan.lnk` |
| Open ssh:// links with Dalan | ☐ | ☐ | §5.1 |
| Start Dalan when I sign in to Windows | ☐ | ☐ | §5.2 |

- **Help text** sits under the last two options (see the mockup). When another app already handles `ssh://` (§5.1 detection), show *"Another app currently opens these links: {name}. Ticking this makes Dalan open them instead."*. If the user has pinned a default in Windows Settings, show instead *"Windows uses {name} for these links. After setup, Dalan can open Settings › Default apps for you."*.
- **Keyboard:** every checkbox is reachable with Tab, Space toggles, and the page title is announced (standard nsDialogs controls).
- **Remembered:** choices are saved (§5.3). On reinstall or update the page is pre-filled with them. With `/UPDATE` (the auto-updater) the page is skipped and the saved choices are kept untouched.

## 5. What the installer writes

`<exe>` is the installed main binary (`{{main_binary_name}}.exe` in the template; check the real name in W1, probably `Dalan.exe`).

### 5.1 ssh:// link handler

**Never** use Tauri's deep-link bundling for this (`plugins.deep-link.desktop.schemes`): the template registers those unconditionally, and ours is opt-in. Don't add `tauri-plugin-deep-link` for Windows.

When the option is ticked, write under **HKCU** in both editions, because it is a per-user choice:

```
HKCU\Software\Classes\ssh                         (default) = "URL:SSH link"   URL Protocol = ""
HKCU\Software\Classes\ssh\DefaultIcon             (default) = "<exe>",0
HKCU\Software\Classes\ssh\shell\open\command      (default) = "<exe>" --open-url "%1"
```

Also register Dalan as a capable app, so it appears in Windows **Settings › Default apps**:

```
HKCU\Software\RegisteredApplications              Dalan = Software\Dalan\Capabilities
HKCU\Software\Dalan\Capabilities                  ApplicationName = Dalan, ApplicationDescription = SSH client
HKCU\Software\Dalan\Capabilities\URLAssociations  ssh = Dalan.ssh
HKCU\Software\Classes\Dalan.ssh\shell\open\command (default) = "<exe>" --open-url "%1"
```

**Detection, before showing page 4:**

- Read `HKCU\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\ssh\UserChoice\ProgId`. If it exists and isn't `Dalan.ssh`, the user has a pinned default that we can't and must not override (it is hash-protected). Show the "Windows uses …" note.
- Otherwise read `HKCU\Software\Classes\ssh\shell\open\command`, then `HKCR\ssh\…`. If either points to another program, show its name (from the file's description, else its file name).

**On uninstall,** remove these keys only if they still point to our `<exe>`. Never delete another app's handler.

### 5.2 Start with Windows

```
HKCU\Software\Microsoft\Windows\CurrentVersion\Run   Dalan = "<exe>" --minimized
```

- HKCU in both editions: the choice belongs to the user who made it.
- Tauri's default uninstaller already deletes this value. It must **not** do so with `/UPDATE`; add a test (§10).

### 5.3 Remembered choices

```
SHCTX\Software\Dalan\Installer   StartMenu, Desktop, SshLinks, Autostart  (DWORD 0/1), Edition ("user"|"allusers"), Version
```

The app reads these too (§9), so its Settings and the installer agree.

### 5.4 Standard keys

Keep Tauri's defaults:

- uninstall key `SHCTX\Software\Microsoft\Windows\CurrentVersion\Uninstall\Dalan` (DisplayName, DisplayVersion, Publisher, DisplayIcon, InstallLocation, UninstallString, QuietUninstallString, EstimatedSize, NoModify/NoRepair)
- the product key

Fill in `bundle.publisher` (§7.2) so "Publisher" isn't empty in Apps & features.

## 6. Uninstaller: Your data

A custom page with two radio buttons (mockup U1). **"Keep my vault and settings" is selected by default.**

- **Keep:** delete only program files, shortcuts, our registry keys (§5) and the uninstall key. Leave `%APPDATA%\app.dalan.desktop` and `%LOCALAPPDATA%\app.dalan.desktop`.
- **Delete everything:**
  - Delete both folders for **the current user**.
  - In the all-users edition, add *"Other people's vaults on this PC are not removed."*
  - Before deleting, if Dalan is running (find the window or process), ask to close it, as Tauri's template already does for the binary.
- **During an update** (`/UPDATE`), the page is skipped and nothing is deleted. This already holds in Tauri's template; keep it.
- **Silent** (`uninstall.exe /S`) keeps data. `/DELETEDATA=1` deletes it.

## 7. Implementation

### 7.1 Files

```
src-tauri/windows/
  installer.nsi            our template: Tauri's default for the pinned CLI version, plus our pages and logic
  installer.upstream.nsi   untouched copy of Tauri's default for that version (for diffing on upgrades)
  UPSTREAM.md              which tauri-cli version / tag it came from, and the list of our changes
  English.nsh              all Tauri English strings with our copy, plus our new strings
  hooks.nsh                NSIS_HOOK_POSTINSTALL / PREUNINSTALL / POSTUNINSTALL: registry and shortcut logic (§5)
  options-page.nsh         page 4 (nsDialogs)
  data-page.nsh            uninstaller page U1 (nsDialogs)
  installer-sidebar.bmp    164x314, 24-bit
  installer-header.bmp     150x57, 24-bit
src-tauri/tauri.windows.conf.json            Windows bundle config (Tauri merges platform files automatically)
src-tauri/tauri.windows-allusers.conf.json   overlay for the all-users edition (installMode, updater endpoint)
scripts/check-nsis-template.mjs              fails when the installed tauri-cli version != UPSTREAM.md version
tests/windows/installer.Tests.ps1            Pester tests (§10)
tests/windows/walk-installer.py              pywinauto walk with screenshots (§10)
```

**Getting the template:**

- Copy `installer.nsi` and `English.nsh` from `tauri-apps/tauri` at the tag matching the `@tauri-apps/cli` version in `pnpm-lock.yaml` (2.12.x today), under `crates/tauri-bundler/src/bundle/windows/nsis/`.
- Keep every `{{handlebars}}` placeholder intact.
- Keep our changes in clearly marked `; --- Dalan ---` blocks, so a later upgrade is a three-way merge (upstream old → upstream new → ours).

### 7.2 Config

`src-tauri/tauri.windows.conf.json`:

```json
{
  "bundle": {
    "publisher": "Mehrdad Rezaee",
    "copyright": "© 2026 Mehrdad Rezaee",
    "shortDescription": "Desktop SSH client",
    "homepage": "https://github.com/OWNER/dalan-app",
    "windows": {
      "webviewInstallMode": { "type": "embedBootstrapper", "silent": true },
      "nsis": {
        "template": "windows/installer.nsi",
        "installerHooks": "windows/hooks.nsh",
        "installMode": "currentUser",
        "languages": ["English"],
        "customLanguageFiles": { "English": "windows/English.nsh" },
        "displayLanguageSelector": false,
        "installerIcon": "icons/icon.ico",
        "headerImage": "windows/installer-header.bmp",
        "sidebarImage": "windows/installer-sidebar.bmp",
        "compression": "lzma",
        "startMenuFolder": null
      }
    }
  }
}
```

Notes on the config:

- Check every key against the schema for the pinned CLI version. Field names were taken from the v2 docs.
- `embedBootstrapper` adds about 1.8 MB and avoids a second download step when WebView2 is missing (rare on Windows 10/11). Offline installs stay a possible later edition (`offlineInstaller`, about 127 MB).
- The publisher name is a placeholder: **ask Mehrdad** (company or personal name). It shows in Apps & features and in the UAC prompt once the build is signed.

`src-tauri/tauri.windows-allusers.conf.json`:

```json
{ "bundle": { "windows": { "nsis": { "installMode": "perMachine" } } },
  "plugins": { "updater": { "endpoints": ["https://github.com/OWNER/dalan-app/releases/latest/download/latest-allusers.json"] } } }
```

### 7.3 Command-line options (for IT and tests)

All of Tauri's options keep working: `/S`, `/P`, `/D=<dir>` (last), `/UPDATE`, `/NS`, `/R`. We add these, parsed in `.onInit` with `GetOptions`:

| option | meaning |
| --- | --- |
| `/STARTMENU=0\|1` | default 1 |
| `/DESKTOP=0\|1` | default 0 |
| `/SSHLINKS=0\|1` | default 0 |
| `/AUTOSTART=0\|1` | default 0 |
| `/DELETEDATA=1` | uninstaller only |

Example: `Dalan_0.1.2_x64-setup.exe /S /DESKTOP=1 /SSHLINKS=1`.

Document these in `replica/launch/release-setup.md`, under "Silent install", for the team.

## 8. CI and updates (`.github/workflows/release.yml`)

1. **Two Windows matrix entries:**
   - `windows` (as today, with `--bundles nsis`)
   - `windows-allusers` (`--bundles nsis --config src-tauri/tauri.windows-allusers.conf.json`)
2. **Rename** the second job's output to `Dalan_<ver>_x64-allusers-setup.exe` and its signature to `.sig` before uploading.
3. **Updater manifests:**
   - `latest.json` points `windows-x86_64` to the just-me setup, as today.
   - A new `latest-allusers.json`, built in the final job from the all-users `.sig` and URL, is attached to the same release.
   - **Check:** the all-users updater uses `/UPDATE` and asks for UAC. Note this in the release notes.
4. **Signing:**
   - Until Azure Artifact Signing is set up, builds are unsigned, so SmartScreen warns.
   - Once it is set up, use `bundle.windows.signCommand` with the Azure signing tool. It must sign **both** setup files **and** `uninstall.exe` (the template's `uninstaller_sign_cmd` / `!uninstfinalize`). Verify that in §10.
5. **Download page:** `site/index.html` Windows card. Main button: the just-me installer. Small link under it: *"Installing for everyone on a shared PC? Get the all-users installer."*

## 9. App changes that go with the installer

1. **`--open-url <url>`** (cold start), and the same argument forwarded by the single-instance plugin to the running app (`tauri_plugin_single_instance::init(|app, args, cwd| …)` already exists in `lib.rs`).
   - Parse it in **Rust**, strictly:
     - scheme `ssh`; `ssh://[user@]host[:port]` only (ignore the optional `;fingerprint=` parameter, never trust it)
     - percent-decode once
     - user: `[A-Za-z0-9._-]{1,64}`, not starting with `-`
     - host: hostname, IPv4 or bracketed IPv6, not starting with `-`
     - port: 1–65535
     - total length ≤ 255
     - anything else is refused with a toast: "This link isn't a valid ssh:// address."
   - Then emit a typed event. The UI opens a New Tab with a **confirmation card**: "Open a connection to deploy@10.0.0.5:22?" with **Connect** and **Cancel**.
   - If a saved host matches user + address + port, show its name and use it. Otherwise "Connect" uses the existing quick-connect path (P27 dedupe rules).
   - **A link never connects by itself**, and never passes options to anything.
   - When the vault is locked, the card waits until it's unlocked.
2. **`--minimized`:** start with the window minimized (used by autostart).
3. **Settings › System** (Windows only): two switches, "Open ssh:// links with Dalan" and "Start Dalan when I sign in to Windows".
   - Commands `system_integration_get` / `system_integration_set` read and write the **same HKCU keys** as §5 (crate `winreg`, Windows-only dependency; ask Mehrdad before adding it, per `CLAUDE.md`), and update `Software\Dalan\Installer`.
   - Show the "Windows uses {app}" note with an **Open Default apps** button (`ms-settings:defaultapps`, opened from Rust).
4. **Settings › About:** edition ("Installed for you" / "Installed for everyone on this PC") and the install folder.
5. **Tests:**
   - Rust unit tests for the URL parser, including the injection cases `ssh://-oProxyCommand=calc@h`, `ssh://h -o x`, `ssh://%2Doops@h`, control characters, overlong input and IPv6.
   - A browser e2e test for the confirmation card (fake backend event).
   - A Rust test that registry writes happen only on Windows (`cfg`).

## 10. Testing

**Pester tests**, `tests/windows/installer.Tests.ps1`, in a new CI job `windows-installer` on `windows-latest` after the build. They also run locally in a Windows VM.

1. **Silent just-me install** with `/DESKTOP=1 /SSHLINKS=1 /AUTOSTART=1`:
   - files are in `%LOCALAPPDATA%\Dalan`
   - both `.lnk` files exist
   - the HKCU `ssh` command points to the exe with `--open-url "%1"`
   - `RegisteredApplications` and Capabilities are present
   - the Run value has `--minimized`
   - the uninstall key fields are filled (Publisher not empty)
   - the `Installer` choices are saved
   - no admin was needed (run as a standard user via `runas /trustlevel:0x20000` or a created test user)
2. **Upgrade:**
   - install the previous release (download the last `vX` from GitHub Releases), then run the new setup with `/UPDATE /P`
   - choices, the Run value and the `ssh` keys survive
   - shortcuts are unchanged; data is untouched (a marker file in `%APPDATA%\app.dalan.desktop`)
3. **Uninstall, keep (`/S`):** program files and our keys are gone; the marker file is still there. A foreign `ssh` handler set before the install is still there.
4. **Uninstall, delete (`/S /DELETEDATA=1`):** both data folders are gone.
5. **All-users edition** (the runner is admin): Program Files, HKLM uninstall key, common Start menu.
6. **Mixed editions:** installing just-me while all-users is installed stops with the message (exit code ≠ 0).
7. **Signing** (only when secrets exist): `Get-AuthenticodeSignature` is `Valid` for both setups and for `uninstall.exe` after install.
8. **The app accepts `--open-url`:** start the installed exe with a valid link and with an injection link. The process starts and there's no crash. Behaviour is covered by the Rust and browser tests.

**Installer walk with screenshots:** `tests/windows/walk-installer.py` (Python + pywinauto, run by `pnpm walk:installer` on Windows and in the CI job).

- It clicks through every page of both editions and the uninstaller, choosing different options each time, and saves a PNG of each page to `test-results/installer-walk/`.
- At the end it builds a gallery (the same gallery script as `pnpm walk`), next to the mockups for comparison.
- In CI it uploads the gallery as an artifact. This is how Mehrdad, on a Mac, sees the real pages.

**Hand check** (in `replica/testing/hand-checks.md`, Windows section; needs a Windows 11 PC or VM, such as Parallels or UTM):

- the pages at 100% and 150% display scaling (bitmaps get stretched by NSIS at 150%; note how it looks)
- SmartScreen behaviour with an unsigned and a signed build
- an `ssh://` link from Edge and from a chat app opens the confirmation card
- autostart after sign-in
- "Default apps" shows Dalan for SSH

## 11. Risks and notes

- **Template upgrades:** every tauri-cli upgrade needs a re-merge of `installer.nsi`. The `check-nsis-template` script in CI fails until `UPSTREAM.md` matches the installed CLI. Keep our changes small and marked.
- **Bitmaps at high DPI:** NSIS Modern UI stretches the BMPs above 100%. If 150% looks soft, the W4 hand check decides whether to ship 2× bitmaps with a DPI switch in `.onGUIInit`. That would be a custom change, so only do it if needed.
- **Per-machine data:** "Delete everything" in the all-users edition only removes the current user's data. That's stated on the page.
- **Antivirus false positives:** unsigned NSIS installers sometimes trigger them. Signing fixes most cases.
- **SSH protocol links are an attack surface:** the strict parser plus the confirmation card is mandatory, and there are no auto-connects. Never add a "don't ask again" for links.

## 12. Copy (all strings, English.nsh)

Use exactly these. They follow the voice in `replica/brand.md`: calm, precise, protective. Keep Tauri's other strings, reworded in the same voice.

| id | text |
| --- | --- |
| welcomeTitle | Welcome to Dalan |
| welcomeText (just me) | This sets up Dalan $VERSION, a calm desktop client for your servers: SSH, files and tunnels in one place.\r\n\r\nIt installs Dalan for you only, without administrator rights. For everyone on this PC, use the all-users installer.\r\n\r\nYour vault, keys and passwords stay on this computer. Setup doesn't touch them. |
| welcomeText (all users) | This sets up Dalan $VERSION for everyone on this PC. Each person keeps their own vault and settings.\r\n\r\nYour vault, keys and passwords stay on this computer. Setup doesn't touch them. |
| optionsTitle / optionsSub | Choose extras / You can change all of these later in Dalan's Settings. |
| optStartMenu | Add a Start menu shortcut |
| optDesktop | Add a desktop shortcut |
| optSsh | Open ssh:// links with Dalan |
| optSshHelp | Links in your browser or chat like ssh://deploy@10.0.0.5 open a new tab in Dalan. Dalan always asks before connecting. |
| optSshOther | Another app currently opens these links: $0. Ticking this makes Dalan open them instead. |
| optSshPinned | Windows uses $0 for these links. After setup, Dalan can open Settings › Default apps for you. |
| optAutostart | Start Dalan when I sign in to Windows |
| optAutostartHelp | Useful if tunnels start when Dalan unlocks. |
| finishTitle / finishText | Dalan is ready / Dalan $VERSION is installed. |
| finishRun | Open Dalan now |
| finishHint | The first time, Dalan asks you to choose a master password. Keep it in your password manager: there's no way to recover it. |
| reinstallTitle / reinstallSub | Dalan is already installed / Version $0 is on this PC. |
| reinstallUpdate | Update to $VERSION (recommended)\r\nKeeps your vault, settings and the extras you chose. |
| reinstallRemove | Remove Dalan first, then install\r\nYour vault and settings are kept unless you choose otherwise. |
| newerInstalled | A newer Dalan ($0) is already installed. Setup won't replace it. |
| otherEditionUser | Dalan is already installed for all users on this PC. Use that copy, or uninstall it first. |
| otherEditionAll | Dalan is already installed for one user on this PC ($0). Uninstall it there first, or use that copy. |
| appRunning | Dalan is open. Close it to continue, then click Retry. |
| unTitle / unSub | Remove Dalan / Choose what happens to your data. |
| unKeep | Keep my vault and settings (recommended)\r\nHosts, keys, passwords, logs and settings stay, so reinstalling picks up where you left off. |
| unDelete | Delete everything\r\nRemoves the encrypted vault, keys, logs and settings from this PC. |
| unDeleteWarn | This can't be undone. Keys that exist only in Dalan are lost, so export any you still need first. |
| unDeleteOthers (all users) | Other people's vaults on this PC are not removed. |
| unDoneKept | Your vault and settings were kept in:\r\n%APPDATA%\app.dalan.desktop\r\n\r\nInstall Dalan again any time to use them. |
| unDoneDeleted | Dalan and your data were removed from this PC. |

## 13. Phases

| phase | scope | done when |
| --- | --- | --- |
| **W1** template and branding | Vendor the template; `UPSTREAM.md` and the check script; artwork; `English.nsh` copy; `tauri.windows.conf.json`; the publisher name (ask); CI builds the just-me installer from the custom template | The CI Windows build is green; the installer shows the new artwork and copy (pywinauto screenshots attached) |
| **W2** pages and logic | Page 4 options, the registry writes (§5), detection, remembered choices, silent options, uninstaller data page, `/UPDATE` keeps everything; the all-users overlay, CI matrix entry, renaming and `latest-allusers.json`; the mixed-edition guard | Pester 1–6 green in CI |
| **W3** app side | `--open-url` parser and confirmation card, `--minimized`, Settings › System switches with `winreg` (ask first), About edition; the download page link | Rust, Vitest and e2e tests green; Pester 8 green |
| **W4** walk and hand check | `walk-installer.py` gallery for both editions plus the uninstaller; signing check once secrets exist; the Windows hand-check section; release notes | Gallery attached to CI; Mehrdad (or a teammate on Windows) has run the hand check |
