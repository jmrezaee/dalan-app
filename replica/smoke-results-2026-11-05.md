# Smoke test results, 2026-11-05 (run by Claude in the real window)

Run against a debug bundle of Dalan (`pnpm tauri build --debug --bundles app`, so the real native window, not a browser stub) with the Docker servers from `scripts/test-servers.sh`. Claude drove the window with computer use. Mehrdad's own vault was moved aside to `~/Library/Application Support/app.dalan.desktop.bak` and a fresh test vault was used.

## Passed in the real window
- **A1 first run and lock:** create vault (8-character rule, strength meter, "no way to recover" note), lock, wrong password message, unlock.
- **A2 F01:** add host, first-time host-key prompt with the right fingerprint, password prompt with Remember, real shell (`TERM=xterm-256color`, colours, 100k lines of output, `stty size` follows the window), close-tab confirmation.
- **A3 F11:** server restart shows "Connection closed"; Reconnect (button) resumes the same tab, scrollback kept, saved password used.
- **A4 F12:** changed key blocks with Saved/Now fingerprints, saved key stays unchanged, "Replace saved key…" needs the typed name, then connects and saves the new key.
- **Jump host (F05):** `inner` (not reachable from the Mac) reached through the bastion; `$HOSTNAME` differs.
- **Keychain:** import a private key from a file; key-only login to the key server works.
- **F03 / settings:** ⌘K palette, ⌘, opens Settings.
- **Import (F10):** SSH config import with duplicates marked, `ProxyCommand` and `Include` warnings, jump chain imported.
- **Port forwarding (F06, local):** rule starts (Active) and `nc 127.0.0.1 2230` returns the `inner` server's SSH banner through the bastion.
- **Logs and history:** a log is written, listed and replays; commands show in the History tab.
- **OS detection:** `os_hint=alpine` stored; cards show the badge after a reload.
- **The app icon** shows in the Dock and the window.

## Failed, and fixed in this phase
1. **A debug bundle crashed at launch** (panic in `lib.rs`): the TypeScript bindings were written to `../app/types` relative to the working directory, which is `/` when launched from Finder. Now anchored to the crate and a failure only logs.
2. **Key-type switch skipped the "identity changed" block** (audit #2, hard rule 5): an ECDSA key where Ed25519 was saved gave the normal "first time" prompt. `known_hosts::check` now returns `Changed` when any other key is saved for the host, and replacing a key deletes the old ones. Test added; re-checked in the window.
3. **Enter didn't activate the default button** on "Connection closed" and "identity changed" (`autofocus` doesn't work on elements added later): the default button now takes focus. Re-checked (Enter closes the tab).
4. **macOS autocorrect capitalised inputs** ("pw" became "Pw" on blur, so "Replace and connect" could never be enabled): `autocapitalize`, `autocorrect` and `spellcheck` are now off on the shared Input, textarea and the other raw inputs. Re-checked: the replace flow works.
5. **Host cards and recent connections were stale after connecting** (OS badge, last connected, saved-password state in the editor): the host list now reloads 1.5 s after a session is ready.
6. **A jump hop's questions were titled with the target's name** ("Password for smoke-inner" asked for the bastion): prompts now use the host named in the question.
7. **Long file paths overflowed the Import key and Certificate dialogs:** truncated.
8. **The sample `ssh_config` was wrong** (`Host *` first overrides everything in real ssh): `replica/smoke/ssh_config_sample` fixed.

## Found, not fixed (to schedule)
- Right-click on a host card opens the browser menu (Look Up, Translate, Inspect Element), not an app menu; card text is selectable.
- The first click on a card's `…` button only shows its tooltip; the second opens the menu.
- Import: a jump host that is unticked as "Already added" is silently dropped from the chain of a ticked host.
- The "Use at least 8 characters." message stays under the first-run password field after it is fixed, until you press Create.
- Windows: typing into masked password fields needed the "show" toggle first (automation limitation, not necessarily a bug).

## Could not be checked by automation (please check by hand)
- SFTP: drag-and-drop between panes, the right-click menu (rename, permissions, delete, Edit in default app) and transfers. A synthetic drag did not start a drag in the web view and right-click on rows showed no menu; this may be an automation limit or a real bug (`FileTable.vue` stops the `contextmenu` event on rows, which would also stop the pane's menu opening).
- Workspaces, snippets, groups and inheritance, Keychain Generate and Install, New Tab screen, SOCKS and remote tunnels, unreachable and auth-failed overlays, keyboard-interactive, and everything in P12/P14 not named above (env vars, proxy, agent, certificates, themes, Telnet, serial, Mosh, AI).


# Second run, 2026-11-05 night (Claude, real window, fresh test vault; the real vault was backed up, moved aside and restored byte for byte)

## Passed in the real window
- **Keychain:** Generate (Ed25519), Install on a server (host-key question, password question, "Key added (folder 700, file 600)", linked to the host), then a key login with no password question.
- **S18 overlays:** auth failed ("Sign-in failed", Edit host opens the editor), unreachable ("Can't reach 127.0.0.1:2299", Retry).
- **Groups and inheritance (F04):** nested groups, "from Production / from Web" hints, group card counts, breadcrumb, connect using the group's user, password and port.
- **SFTP:** right-click menu (after fix), Rename, Permissions (600 shows as `-rw-------`), Delete with confirmation, New folder, Copy to other pane, conflict row (Skip / Keep both / Replace) with `index (copy).html`, Edit in default app (TextEdit opened, local save detected, Upload wrote the new content to the server).
- **Snippets and workspaces:** create a snippet, Run on hosts (3 panes, the unreachable one shows its own failure), broadcast (⌘B skips the dead pane and says so), Save as template, reopen from New tab.
- **P16 in the window:** unlock, terminals, SFTP, workspaces all work under the strict CSP; the data folder is 0700 and `vault.db*`, log and edit copies are 0600; a second launch of the binary exits and the first keeps running.

## Bugs found and fixed
1. **SFTP right-click menu never opened** (`FileTable.vue` stopped the `contextmenu` event on rows, so the pane's menu never saw it). Rows now let it bubble; blank-space clicks still clear the target.
2. **No toast was ever visible:** `vue-sonner` v2 needs `import 'vue-sonner/style.css'`. Without it "Saved", "Uploaded", errors and the "was saved / Upload" prompt were invisible. Imported in `Sonner.vue`.
3. **Run snippet on hosts typed nothing:** the shell's `ready` event can arrive before `session_open` returns the session id, so the first write was dropped and the snippet forgotten. The store now waits for both (`runAutorun`).
4. **Possible fix, not verified:** `dragDropEnabled: false` on the window, plus `select-none` and `-webkit-user-drag: element` on file rows, so HTML5 drag and drop can start in WKWebView.

## Still not checked
- **Drag and drop** (SFTP rows between panes, host card onto a group). A scripted mouse drag does not start a drag in the web view, with or without the fixes above. It needs a human hand.
- Big-file transfer cancel (C2.5), SOCKS/remote tunnels, B1.4 slow connect, keyboard-interactive, P12/P14 extras (Telnet, serial, Mosh, AI, themes).
- `open -n` (a forced second copy through LaunchServices) did not make the second copy exit and no socket file appeared. Starting the binary twice from a shell works. A normal double-click on the icon reuses the running app on macOS, so this may be harmless; please try it.

## Small things noticed
- The "Saved." toast sits on top of the editor sheet's Save/Cancel footer.
- After an upload the remote file list still shows the old size until refreshed.
- The first click after the window regains focus is sometimes swallowed.
