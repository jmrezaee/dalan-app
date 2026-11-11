# Where the installer template came from

`installer.nsi` and `English.nsh` here are Dalan's copies of Tauri's default NSIS files. Our changes sit between
`; --- Dalan ---` and `; --- /Dalan ---` in `installer.nsi`, so an upgrade is a three-way merge:
`installer.upstream.nsi` (the old default) → the new default → `installer.nsi` (ours).

```
tauri-cli: 2.12.4
tag: tauri-cli-v2.12.4 (github.com/tauri-apps/tauri)
files: crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi, .../languages/English.nsh
installer.upstream.nsi sha256: d00089d3b1a2c4a30d7b6e098079100cb8f25e980f84d4515aca4597b95e967d
English.upstream.nsh sha256: e663ba48d3c4248d77d447766d935d5ad0e9122bb452e2ee75ddf9b59975f141
```

`scripts/check-nsis-template.mjs` (`pnpm check:nsis`, run in CI) fails when the `@tauri-apps/cli` version in
`pnpm-lock.yaml` is not the `tauri-cli` version above, or when an `*.upstream.*` copy was edited.

To upgrade: fetch the two files at the new tag, merge them into `installer.nsi` and `English.nsh`, overwrite the
`*.upstream.*` copies, then update the version and the two hashes above (`shasum -a 256`).

## Our changes (W1)

- **Welcome page:** our title and text; the all-users edition (`installMode: perMachine`) gets its own text.
- **Reinstall page ("Already installed"):** our texts, the installed version in the sub title, a line of
  explanation under each choice, and for an upgrade the first choice is now "Update" (keeps everything) and the
  second "Remove Dalan first". `PageLeaveReinstall` reads the same order.
- **Finish page:** our title, text, hint and "Open Dalan now".
- **`RestorePreviousInstallLocation`:** also reads `Software\dalan\Dalan`, where 0.1.x releases (no publisher set)
  kept the install folder. Without it an update would forget a custom folder.
- **`English.nsh`:** every upstream string id kept, reworded; new ids for the pages above. The spec's
  `appRunning` text ("click Retry") was changed: the template's prompt has OK and Cancel, no Retry.

## Our changes (W2)

All the new logic is in our own files (`options-page.nsh`, `data-page.nsh`, `hooks.nsh`), so `installer.nsi` only
gets small, marked edits:

- `Var`s and `!include` of `options-page.nsh` after the template's `Var`s; `DALAN_DIR` comes from `hooks.nsh`
  (`__FILEDIR__`), because the bundler copies the template elsewhere.
- `Page custom DalanOptionsPage DalanOptionsLeave` between the folder page and the installing page.
- `.onInit` calls `DalanGuardEdition` (one edition per PC) and `DalanInitOptions` (defaults, saved choices, command line).
- The finish page lost its desktop-shortcut button (it is a choice on the extras page now).
- `CreateOrUpdateDesktopShortcut` is called in every mode; both shortcut functions return when the choice is off.
- The uninstaller's confirm page (with the "Delete app data" checkbox) is replaced by `un.DalanDataPage`, which sets
  the same variable; a done page (`MUI_UNPAGE_FINISH`) was added; `un.onInit` reads `/DELETEDATA=1`.
- Hooks: `NSIS_HOOK_POSTINSTALL` calls `DalanApplyOptions`, `NSIS_HOOK_POSTUNINSTALL` calls `un.DalanPostUninstall`.
- Strings: the extras page, log lines, the data and done pages, the one-edition messages. The spec's "Another app
  currently opens these links: {name}." became "{name} currently opens these links." because NSIS reads `$Var.` as a
  variable name that includes the dot.
