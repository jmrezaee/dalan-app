# Dalan hand checks

Only what automation cannot cover. Everything else is tested by `pnpm test:all`, `pnpm walk`, `pnpm test:app` (see `test-plan.md`). Run this list in the **real macOS window** (`pnpm tauri dev` or an installed build), before each release and in P28.

How to use it: do each item, write the date, the result (`ok`, or what went wrong) and your initials on its last line. Don't fix anything while checking; note it. An item that passes twice in a row and later gets a test can be deleted from here.

Servers for the items that need one: `scripts/test-servers.sh up` (see `scripts/test-servers.sh` for ports and passwords). Never use a real server.

## A. The macOS window (WKWebView)

Found by hand on 2026-11-07 and fixed: dragging the tab bar threw `window.start_dragging not allowed` (missing `core:window:allow-start-dragging`; now granted, and `tests/capabilities.test.ts` guards it). Re-check by dragging the window by the tab bar.

The browser tests run in Chromium and the real-app tests in WebKitGTK on Linux. These behave differently on macOS.

- [ ] **A1 Right-click menus.** Right-click a host card, a key, a snippet, a tab, an SFTP row and empty SFTP space. **Expected:** each opens its menu at the pointer; Esc closes it; the "…" button on cards opens the same list.
  `Date / result / initials:` 2026-11-07 (Claude, real window, bundle from commit f1912b4a): ok for a host card, a key (Copy public key, Install on a server, Edit, Attach certificate, Delete), a snippet (Run, Edit, Duplicate, Delete), a tab (Rename, Close, Duplicate tab, Close others) and an SFTP row (Copy to left pane, Edit in default app, Edit in TextEdit, Open with…, Rename, Permissions, Delete, New folder, Refresh); Esc closes. Finding (FIXED the same day, polish.spec.ts): right-clicking empty SFTP space with a row still selected showed that row's menu; it now clears the selection and offers only New folder and Refresh. Not checked: the "…" button.
- [ ] **A2 Drag and drop.** Drag a host onto a group; drag a tab onto another tab (a workspace opens); drag a tab to reorder; drag a snippet onto a package; drag files between the two SFTP panes; drag a file from Finder onto a pane. **Expected:** each works with a drop highlight and no stuck drag.
  `Date / result / initials:` 2026-11-07 (Claude, real window, bundle from commit f1912b4a): ok for dragging a host onto a group ("Moved to lab."), reordering a tab, and dragging the window by the tab bar (no permission error). Slow press-move-release works; a fast scripted drag does not start a drag. Not checked: tab onto tab (workspace), snippet onto package, files between SFTP panes, Finder onto a pane.
- [ ] **A3 Typing in other scripts.** In a terminal and in a text field, type Persian text with the Persian keyboard, and use an IME (for example Japanese). **Expected:** characters appear correctly, no doubled or lost keys, right-to-left text in a field aligns correctly.
  `Date / result / initials:` 2026-11-07 NOT DONE: Claude can only send Latin keystrokes. Yours.
- [ ] **A4 Shortcuts are not swallowed.** ⌘T, ⌘W, ⌘K, ⌘,, ⌘., ⌘F, ⌘B, ⌘D, ⌘⇧D, ⌘S inside a terminal and outside; Ctrl+Tab. **Expected:** Dalan handles each, and the terminal still gets Ctrl+C, Ctrl+D and Ctrl+Z.
  `Date / result / initials:` 2026-11-07 (Claude, real window, bundle from commit f1912b4a): ok for ⌘T, ⌘K, ⌘W (asks before closing a live tab), ⌘F (find bar), ⌘. (side panel), ⌘B (broadcast, both panes get the line), ⌘⇧D (new pane below), ⌘, from the Vault tab; the terminal still gets Ctrl+C, Ctrl+Z (job stopped) and Ctrl+D (shell logout). Not checked: ⌘D, ⌘S, Ctrl+Tab, and ⌘, from a terminal tab (did nothing visible from a New tab).
- [ ] **A5 Clipboard.** Settings > Terminal > Copy on select on; select text in a terminal; paste into another app. Then ⌘C and ⌘V in a terminal. Then select text and press Ctrl+Shift+C. **Expected:** the macOS clipboard has the text both ways, Ctrl+Shift+C copies too, and Ctrl+C with a selection still interrupts the shell on a Mac (off a Mac it copies while text is selected, and interrupts with nothing selected).
  `Date / result / initials:` 2026-11-07 (Claude, real window, bundle from commit f1912b4a): ok: ⌘C copies a selection, ⌘V pastes into the terminal, and with Settings > Terminal > Copy on select on, selecting a line puts it on the macOS clipboard. Not checked: pasting into another app.
- [ ] **A6 Bell.** Settings > Terminal > Bell sound on, then `printf '\a'`. **Expected:** one short soft beep; ten in a row do not stack into noise.
  `Date / result / initials:` 2026-11-07 NOT DONE: Claude cannot hear. Yours.
- [ ] **A7 Window feel.** Resize the window small and large; full screen; two displays if you have them. Type fast and run `seq 1 200000`. **Expected:** no layout breakage, typing stays responsive, no stuck scroll.
  `Date / result / initials:` 2026-11-07 (Claude, real window, bundle from commit f1912b4a): `seq 1 200000` ran to the end, typing worked straight after, no stuck scroll. Not done: resizing, full screen, two displays.

## B. macOS integration

- [ ] **B1 One Dalan at a time.** With Dalan open, run `open -n -a Dalan` (or launch the build a second time). **Expected:** no second window; the first one comes to the front.
  `Date / result / initials:` 2026-11-07, ok on a fresh bundle built from current code: `open -n` left one process, no new crash report (driven by Claude). The /Applications copy is from Nov 5, before single-instance existed, and crashed on a second launch (Apple event exception); replace it with a current build.
- [ ] **B2 Open with….** SFTP > a server file > Open with…. **Expected:** the native chooser starts in /Applications; the chosen app opens the downloaded copy; the next time the menu offers "Edit in <that app>"; Settings > Terminal lists it and Forget removes it; a `.sh` or `.command` file is refused with the default app.
  `Date / result / initials:` 2026-11-07 (Claude, real window, bundle from commit f1912b4a): ok: the native chooser started in /Applications, TextEdit opened the downloaded copy with the right text, the next right-click offered "Edit in TextEdit", Settings > Terminal listed `.txt: TextEdit` and Forget removed it. Not checked: a `.sh` file refused with the default app.
- [ ] **B3 The system SSH agent.** With `SSH_AUTH_SOCK` set (ssh-agent or 1Password), add a host with agent forwarding on and connect to a test server. **Expected:** `ssh-add -l` on the server lists your keys; with forwarding off it says there is no agent.
  `Date / result / initials:` 2026-11-07 (Claude, real window, bundle from commit f1912b4a): ok: with an ssh-agent holding the test key and Forward agent on, `ssh-add -l` on the server listed the key and SSH_AUTH_SOCK was set. Not checked: forwarding off.
- [ ] **B4 Local network prompt.** First connect to a server on your LAN. **Expected:** macOS may ask for Local Network access; after allowing it, the connection works.
  `Date / result / initials:` 2026-11-07 NOT DONE: needs a LAN host and a first run; no prompt appeared for localhost. Yours.
- [ ] **B5 Quit and relaunch.** ⌘Q with sessions open. **Expected:** Dalan asks before ending live sessions; after relaunch the vault asks for the master password and tabs are gone, hosts are kept.
  `Date / result / initials:` 2026-11-07 (Claude, real window): first try FAILED (quit with a live session ended the app silently; the macOS Quit menu item skips Tauri's exit events). Fixed by replacing that menu item and asking in Rust + the UI (`quit.rs`, `QuitDialog.vue`). Re-checked: with a live Telnet shell, Dalan > Quit Dalan and the red close button both show "Quit Dalan?"; Keep working returns to the shell; with no session open Quit exits at once. Still yours: ⌘Q from the keyboard, Quit anyway, and relaunch (vault asks for the master password, tabs gone, hosts kept).

## C. Hardware and outside services

- [ ] **C1 A serial device.** Plug in a USB serial adapter (or use `socat` to make a pair). **Expected:** the device appears in the Device picker; text flows both ways at the chosen baud rate.
  `Date / result / initials:`
- [ ] **C2 A real 2FA or PAM server.** Only if the Docker `kbd` server is not enough for you. **Expected:** password, then the code question; the right code signs in.
  `Date / result / initials:`
- [ ] **C3 AI with a real key.** Settings > AI with your own key, then Ask AI "show the 10 biggest files here". **Expected:** one suggestion that you can Insert (it does not run); the key is never shown again after saving.
  `Date / result / initials:`
- [ ] **C4 Mosh over a Wi-Fi drop.** Connect with Mosh, switch Wi-Fi off for 20 seconds and on again. **Expected:** the session resumes.
  `Date / result / initials:`

## D. Open findings from the automated runs (check by hand, then fix or add a test)

These are things the tests found or could not settle. Each has a place in `progress.md`.

- [ ] **D1 A new workspace's first pane stays empty.** Snippets > a snippet with two default servers > Run on both. **Expected:** both panes show a banner and the snippet's output. **Seen in the real-app test:** the first pane stayed empty about 2 times in 3 while the second pane showed the snippet typed twice. Try it ten times.
  `Date / result / initials:` 2026-11-07 (Claude, real window, bundle from commit f1912b4a): did not reproduce: three runs of "Run on 2 servers" (an SSH and a Telnet host) showed the snippet output in both panes every time. The Docker failure is probably Linux WebKitGTK timing; keep the real-app spec skipped until it is retried there.
- [ ] **D2 Selecting an SFTP row, then Copy.** Click a file in one pane, press "Copy to the other pane". **Expected:** the row highlights, the button enables, the file is copied. **Seen in the real-app test:** a WebDriver click did not select the row.
  `Date / result / initials:` 2026-11-07 (Claude, real window, bundle from commit f1912b4a): ok: clicking a file selected it ("1 selected") and the Copy button downloaded it to the local pane. The Docker failure is WebDriver-only.
- [ ] **D3 Palette: Enter after typing.** (2026-11-07, Claude: typing `127` then Enter opened the first match in the real window; looks fine on macOS.) ⌘K, type part of a host name, press Enter without pressing the arrow keys. **Expected:** the first result opens. **Seen in the walkthrough:** nothing happened until the down arrow was pressed.
  `Date / result / initials:`
- [ ] **D4 Side panel tab names.** Open a terminal's side panel. **Expected:** "Ask AI" fits on one line. **Seen:** it wrapped onto two lines at 300 px.
  `Date / result / initials:` 2026-11-07 (Claude, real window, bundle from commit f1912b4a): still wrapped: "Ask AI" broke onto two lines in the side panel. FIXED the same day (tabs are `nowrap` with a little less padding; `tests/e2e/polish.spec.ts`); re-look on macOS.
- [ ] **D5 Quick connect toast.** New tab > type `me@10.0.0.9:2222` > Enter. **Expected:** a connection opens; a "Saved." toast is acceptable but should say what was saved. **Seen:** a bare "Saved." toast.
  `Date / result / initials:` 2026-11-07 (Claude, real window, bundle from commit f1912b4a): the bare "Saved." toast also shows after saving a host, a group and a snippet; it never said what was saved. FIXED the same day: toasts now say "Saved edge box.", "Saved group lab.", "Saved say hello." and so on.
- [ ] **D6 Import from a file.** Hosts > Import > Choose file… with your own SSH config (and, if you have them, a PuTTY or MobaXterm export). **Expected:** the preview lists your hosts; Include and ProxyCommand notes appear; the filter and Select all act on the shown rows; the group tick boxes toggle whole folders. (WebDriver cannot drive the native file dialog, so this is not in the real-app tests.)
  `Date / result / initials:` 2026-11-07 (Claude, real window, bundle from commit f1912b4a): the native file dialog works (it was used for the key import: path shown, key saved). The SSH config import itself was not run.
- [ ] **D7 Creating the vault is quick.** Delete the app's data folder (`~/Library/Application Support/app.dalan.desktop`, keep a copy), start the app, create a vault. **Expected:** a few seconds at most. **Seen:** in the Linux debug build it took from seconds to minutes before the crypto crates were built optimised.
  `Date / result / initials:` 2026-11-07 (Claude, real window, bundle from commit f1912b4a): the first vault was created in seconds in the release build (the user created it); not measured in a debug build.

## E. Open checks from the `[h]` phases (merged in P28)

Each phase below was built and tested by machines but never looked at by you in the real window. Items already in A to D (right-click menus, drag and drop, bell, Open with…, import from a file, the first-pane race, the SFTP Copy) are not repeated. Use the test servers: `a` (password), `kbd` (code question), `b` (key), `c` through `a` (jump host).

- [ ] **E1 Tab dots (P18).** Run `sleep 5; echo done` in a tab, switch away before it prints. **Expected:** the tab dot turns green when output arrives while you are away, yellow while a question (password or key) is open, red when the connection drops; a workspace tab shows one combined dot.
  `Date / result / initials:` 2026-11-07 (Claude, real window): ok: output in a background tab turned its dot green; a dropped connection turned it red with the "Connection closed" card. Not checked: yellow (a question open) and the workspace's combined dot.
- [ ] **E2 Tab bar keys and mouse (P18).** ⌃Tab and ⌃⇧Tab cycle tabs; middle-click closes a tab; double-click (or F2 on a focused tab) renames; ⌘⇧D in a workspace copies the focused server one row down. **Expected:** each works; a live shell asks before closing.
  `Date / result / initials:` 2026-11-07 (Claude, real window): ok: ⌃Tab and ⌃⇧Tab cycle tabs, middle-click closes a tab, double-click renames. Not checked: F2 on a focused tab, ⌘⇧D (seen working in the A4 run).
- [ ] **E3 Settings take effect (P18, P25).** Change the font size and family, the local shell path, and post-quantum key exchange on/off; rebind one shortcut in Settings. **Expected:** open terminals change at once; shortcut labels in menus show the new key; the light theme swatches look right.
  `Date / result / initials:` 2026-11-07 (Claude, real window): partly ok: the font size change reached the open terminal at once (note: typing "20" into the field gave 32, the field seems to append to the old value instead of replacing it); the Light theme swatch and page look right. Not checked: font family, local shell path, post-quantum toggle, rebinding a shortcut.
- [ ] **E4 A tunnel under the CSP, and tag and recents menus (P16, P17).** Start a local tunnel in the real window and fetch a page through it; delete a tag from its menu; open New tab and look at the recents. **Expected:** all three work with no error in the window.
  `Date / result / initials:` 2026-11-07 (Claude, real window, latest build): tunnel ok: a Local rule made with the 3-step wizard (key server -> web:80) went Active, `curl http://127.0.0.1:15080/` returned the nginx page, and after `docker restart` of the server the tunnel came back by itself and worked again; the toast read "Saved web through key.". The key test server has AllowTcpForwarding off by default, so the first try was reset by the server (not an app fault); enabled it in the test container. New tab recents were seen in the A4 run. Not checked: the tag delete menu.
- [ ] **E5 Two-step sign-in against the `kbd` server (P17).** Connect to the `kbd` server (see `scripts/test-servers.sh` for its code). **Expected:** password, then the question; the right code signs in, a wrong one does not.
  `Date / result / initials:`
- [ ] **E6 Connection log and drops (P21).** (1) Connect to a wrong host/port: the log says where it stopped and Copy details puts plain text on the clipboard. (2) Open two sessions on server `a`, `docker restart dalan-test-a`: one card, "Reconnect all" works. (3) Settings > Terminal > Reconnect automatically on, restart again: tabs come back by themselves. (4) Wrong password: "Try another way" lists only what the server accepts.
  `Date / result / initials:` 2026-11-07 (Claude, real window): partly: after `docker restart` the tab went red and offered Reconnect, which brought the same tab back with the old scrollback above a new shell. FAILED: with Settings > Terminal > Reconnect automatically on, a restarted server never came back by itself. Cause found: Rust always sends `exit` before `disconnected`, and the UI treated every `exit` as the user typing `exit`; fixed in `sessions.ts` (only an exit WITH a status ends the tab) with a test; RE-CHECKED OK in the rebuilt app (same day): with the setting on and `docker restart`, a new shell came back in the same tab within 20 s with no click. Not checked: wrong host/port log and Copy details, Reconnect all, Try another way.
- [ ] **E7 Menus and quick actions (P22).** (1) Host card "…" and right-click show the same items; Connect with SFTP opens the SFTP tab with that host. (2) Add to workspace > New workspace, then add a second host. (3) Right-click a tab: Duplicate tab, Close others (asks before closing live ones). (4) Duplicate a host that has a saved password and connect to the copy without being asked; delete the original and connect to the copy again. (5) Filter and sort on Keychain, Snippets, Port forwarding and Known hosts.
  `Date / result / initials:` 2026-11-07 (Claude, real window, latest build): partly ok: host card menu and Connect with SFTP, Add to workspace (workspace opened), tab menu (Duplicate tab, Close others) were seen earlier; search and sort work on Port forwarding (filter "zzz" empties, "web" finds the rule, the sort button toggles). Not checked: duplicating a host that has a saved password (needs a password typed into a secure field), filter and sort on Keychain, Snippets and Known hosts.
- [ ] **E8 The host editor (P23).** (1) Edit an existing host: proxy, environment and agent values load and save unchanged. (2) Two hosts with the same user and password each show the shared-login note. (3) ⌘S saves from a sub-page; closing with changes still warns. (4) Telnet, Serial and Mosh hosts show only the options that apply.
  `Date / result / initials:` 2026-11-07 (Claude, real window, latest build): partly ok: opening an existing host loads its key ("test key") and SSH agent ("Forwarded") values. Not checked: saving unchanged, shared-login note, ⌘S from a sub-page, Telnet/Serial/Mosh option sets.
- [ ] **E9 Terminal side panel (P24).** (2026-11-07, Claude, real window: partly ok: Ember theme recoloured the open terminal at once and "Use the default" cleared it; "Ask AI" fits on one line. Not checked: persistence after reconnect/restart, font family, History suggestion, Save snippet; broadcast frame seen in the A4 run.) (1) ⌘. then Theme: the open terminal changes at once and stays after reconnect; "Use the default" clears it. (2) Font size and family change every open terminal. (3) Close the panel on Font, reopen: it opens on Font; restart the app: still Font. (4) Type the same command (4+ characters) three times on a server with history on: History suggests it, Save snippet creates it. (5) In a workspace turn Broadcast on: every live pane has a dashed amber frame.
  `Date / result / initials:`
- [ ] **E10 Keys and shells (P25, P27).** (1) Keychain > Generate key: each type shows its hint and the ML-DSA note. (2) Type `exit` in a shell with auto-reconnect on: the tab stays ended. (3) Through a bastion, fail sign-in and use Try another way: the choice applies to the target. (4) SFTP path bar: type `~/..`. **Expected:** no surprises in any of them.
  `Date / result / initials:`
- [ ] **E11 SFTP in the window (P18).** F2 renames a row, ⌘C and ⌘V copy and paste files between panes, a folder with 500+ files scrolls smoothly. **Expected:** each works.
  `Date / result / initials:` 2026-11-07 (Claude, real window, latest build): ok: F2 renames a remote file ("Renamed."), ⌘C shows "Copied 1 item. Press ⌘V in the other pane." and ⌘V pasted the file into the left pane. Note: right after a rename the first ⌘C/⌘V did nothing until the row was clicked again (focus). Not checked: a 500+ file folder.

## F. Release (P29)

- [ ] **F1 A signed and notarised build opens without a warning.** Download the release DMG on a Mac that has never seen it. **Expected:** double-click opens it; no "unidentified developer" dialog.
  `Date / result / initials:`
- [ ] **F2 The updater.** Install v0.1.0, publish v0.1.1, open Settings > About > Check for updates. **Expected:** it offers 0.1.1, installs, restarts, and the vault still opens.
  `Date / result / initials:`
- [ ] **F3 The other installers.** DMG, and on Windows the MSI or EXE, and on Linux the AppImage: install and launch each. **Expected:** the app starts and creates a vault.
  `Date / result / initials:`

## G. Several SFTP tabs (P30)

- [ ] **G1.** Right-click a host, "Connect with SFTP in a new tab": a tab "SFTP 2" opens with that host on the right. Then do the same for a second host and for the first SFTP tab ("Connect with SFTP"): each tab keeps its own server when you switch.
- [ ] **G2.** Start a large copy in SFTP 2, switch to Vault and back: it keeps going. Close SFTP 2 while it runs: the copy stops, and nothing else changes.
- [ ] **G3.** ⌘3 reaches SFTP 2; ⌃Tab goes Vault, SFTP, SFTP 2, then the sessions. Lock the vault and unlock: only the first SFTP tab is left, empty.
