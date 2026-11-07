# Dalan hand checks

Only what automation cannot cover. Everything else is tested by `pnpm test:all`, `pnpm walk`, `pnpm test:app` (see `test-plan.md`). Run this list in the **real macOS window** (`pnpm tauri dev` or an installed build), before each release and in P28.

How to use it: do each item, write the date, the result (`ok`, or what went wrong) and your initials on its last line. Don't fix anything while checking; note it. An item that passes twice in a row and later gets a test can be deleted from here.

Servers for the items that need one: `scripts/test-servers.sh up` (see `scripts/test-servers.sh` for ports and passwords). Never use a real server.

## A. The macOS window (WKWebView)

The browser tests run in Chromium and the real-app tests in WebKitGTK on Linux. These behave differently on macOS.

- [ ] **A1 Right-click menus.** Right-click a host card, a key, a snippet, a tab, an SFTP row and empty SFTP space. **Expected:** each opens its menu at the pointer; Esc closes it; the "…" button on cards opens the same list.
  `Date / result / initials:`
- [ ] **A2 Drag and drop.** Drag a host onto a group; drag a tab onto another tab (a workspace opens); drag a tab to reorder; drag a snippet onto a package; drag files between the two SFTP panes; drag a file from Finder onto a pane. **Expected:** each works with a drop highlight and no stuck drag.
  `Date / result / initials:`
- [ ] **A3 Typing in other scripts.** In a terminal and in a text field, type Persian text with the Persian keyboard, and use an IME (for example Japanese). **Expected:** characters appear correctly, no doubled or lost keys, right-to-left text in a field aligns correctly.
  `Date / result / initials:`
- [ ] **A4 Shortcuts are not swallowed.** ⌘T, ⌘W, ⌘K, ⌘,, ⌘., ⌘F, ⌘B, ⌘D, ⌘⇧D, ⌘S inside a terminal and outside; Ctrl+Tab. **Expected:** Dalan handles each, and the terminal still gets Ctrl+C, Ctrl+D and Ctrl+Z.
  `Date / result / initials:`
- [ ] **A5 Clipboard.** Settings > Terminal > Copy on select on; select text in a terminal; paste into another app. Then ⌘C and ⌘V in a terminal. **Expected:** the macOS clipboard has the text both ways.
  `Date / result / initials:`
- [ ] **A6 Bell.** Settings > Terminal > Bell sound on, then `printf '\a'`. **Expected:** one short soft beep; ten in a row do not stack into noise.
  `Date / result / initials:`
- [ ] **A7 Window feel.** Resize the window small and large; full screen; two displays if you have them. Type fast and run `seq 1 200000`. **Expected:** no layout breakage, typing stays responsive, no stuck scroll.
  `Date / result / initials:`

## B. macOS integration

- [ ] **B1 One Dalan at a time.** With Dalan open, run `open -n -a Dalan` (or launch the build a second time). **Expected:** no second window; the first one comes to the front.
  `Date / result / initials:`
- [ ] **B2 Open with….** SFTP > a server file > Open with…. **Expected:** the native chooser starts in /Applications; the chosen app opens the downloaded copy; the next time the menu offers "Edit in <that app>"; Settings > Terminal lists it and Forget removes it; a `.sh` or `.command` file is refused with the default app.
  `Date / result / initials:`
- [ ] **B3 The system SSH agent.** With `SSH_AUTH_SOCK` set (ssh-agent or 1Password), add a host with agent forwarding on and connect to a test server. **Expected:** `ssh-add -l` on the server lists your keys; with forwarding off it says there is no agent.
  `Date / result / initials:`
- [ ] **B4 Local network prompt.** First connect to a server on your LAN. **Expected:** macOS may ask for Local Network access; after allowing it, the connection works.
  `Date / result / initials:`
- [ ] **B5 Quit and relaunch.** ⌘Q with sessions open. **Expected:** Dalan asks before ending live sessions; after relaunch the vault asks for the master password and tabs are gone, hosts are kept.
  `Date / result / initials:`

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
  `Date / result / initials:`
- [ ] **D2 Selecting an SFTP row, then Copy.** Click a file in one pane, press "Copy to the other pane". **Expected:** the row highlights, the button enables, the file is copied. **Seen in the real-app test:** a WebDriver click did not select the row.
  `Date / result / initials:`
- [ ] **D3 Palette: Enter after typing.** ⌘K, type part of a host name, press Enter without pressing the arrow keys. **Expected:** the first result opens. **Seen in the walkthrough:** nothing happened until the down arrow was pressed.
  `Date / result / initials:`
- [ ] **D4 Side panel tab names.** Open a terminal's side panel. **Expected:** "Ask AI" fits on one line. **Seen:** it wrapped onto two lines at 300 px.
  `Date / result / initials:`
- [ ] **D5 Quick connect toast.** New tab > type `me@10.0.0.9:2222` > Enter. **Expected:** a connection opens; a "Saved." toast is acceptable but should say what was saved. **Seen:** a bare "Saved." toast.
  `Date / result / initials:`
- [ ] **D6 Import from a file.** Hosts > Import > Choose file… with your own SSH config (and, if you have them, a PuTTY or MobaXterm export). **Expected:** the preview lists your hosts; Include and ProxyCommand notes appear; the filter and Select all act on the shown rows; the group tick boxes toggle whole folders. (WebDriver cannot drive the native file dialog, so this is not in the real-app tests.)
  `Date / result / initials:`
- [ ] **D7 Creating the vault is quick.** Delete the app's data folder (`~/Library/Application Support/app.dalan.desktop`, keep a copy), start the app, create a vault. **Expected:** a few seconds at most. **Seen:** in the Linux debug build it took from seconds to minutes before the crypto crates were built optimised.
  `Date / result / initials:`

## E. Open checks from earlier phases

P16 to P18 and P21 to P27 each left a numbered "Hand checks" list in `progress.md`. P28 merges the ones not covered above into this file and asks you to run them. Until then they are the authority.

## F. Release (P29)

- [ ] **F1 A signed and notarised build opens without a warning.** Download the release DMG on a Mac that has never seen it. **Expected:** double-click opens it; no "unidentified developer" dialog.
  `Date / result / initials:`
- [ ] **F2 The updater.** Install v0.1.0, publish v0.1.1, open Settings > About > Check for updates. **Expected:** it offers 0.1.1, installs, restarts, and the vault still opens.
  `Date / result / initials:`
- [ ] **F3 The other installers.** DMG, and on Windows the MSI or EXE, and on Linux the AppImage: install and launch each. **Expected:** the app starts and creates a vault.
  `Date / result / initials:`
