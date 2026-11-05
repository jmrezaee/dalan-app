# Dalan smoke test (real window)

Goal: prove, with your own eyes, that Dalan works against real SSH servers. Nothing from P2 on has been run in the real Tauri window before. Do the parts in order; **Part A is the M1 gate** (about 15 minutes). The whole list is about 60 minutes.

Write down every step that does not do what is expected. Don't fix anything yourself, just note it. At the end, paste the short report (see the bottom) back to Claude.

## 0. Setup (5 minutes)

1. Start Docker Desktop.
2. In the project folder:

   ```bash
   scripts/test-servers.sh up
   scripts/test-servers.sh status
   ```

   You should see three servers running (`a`, `b`, `c`) and host-key fingerprints for each. Restarting a server takes a few seconds.
3. Start the app (Node 24.12.0):

   ```bash
   nvm use 24.12.0
   pnpm tauri dev
   ```

   The first build can take a few minutes. A window called **Dalan** opens.

   If port 4000 is taken (for example by another Docker container), run it on 4001 instead, without changing any file:

   ```bash
   pnpm tauri dev --config '{"build":{"devUrl":"http://127.0.0.1:4001","beforeDevCommand":"pnpm dev --port 4001"}}'
   ```
4. Servers, for reference:

   | name | address | user | sign-in |
   | --- | --- | --- | --- |
   | password server (bastion) | `127.0.0.1` port `2222` | `test` | password `test` |
   | key server | `127.0.0.1` port `2223` | `keyuser` | key only. Keys are in `.test-servers/` (`id_ed25519`, and `id_ed25519_passphrase` with passphrase `passphrase`) |
   | inner server | `inner` port `2222` | `test` | password `test`. Only reachable through the bastion |

## Part A. M1 gate: F01, F11, F12 (about 15 minutes)

### A1. First run (S24)

If you have run Dalan on this Mac before, a vault already exists and you will see the **unlock** screen instead of "create". For a clean test, quit Dalan and move the old data aside (nothing is deleted):

```bash
mv ~/Library/Application\ Support/app.dalan.desktop ~/Library/Application\ Support/app.dalan.desktop.bak
```

Then start the app again. (To go back later: quit, delete the new folder and move the `.bak` one back.)

- [ ] **A1.1** The window shows the "create a master password" screen. **Expected:** it says the password can't be recovered (note the exact sentence), has a strength meter, and the create button is disabled until the password has at least 8 characters.
- [ ] **A1.2** Create the vault with a password you'll remember (for example `smoke-test-1`). **Expected:** the Hosts screen opens with the empty state "No servers yet".
- [ ] **A1.3** Press the lock button (bottom left), then enter a wrong password twice, then the right one. **Expected:** wrong ones shake and say it didn't work; the right one unlocks.

### A2. Add a host and connect with a password (F01, S02/S03/S18)

- [ ] **A2.1** Add host: Label `pw`, Address `127.0.0.1`, Port `2222`, Username `test`, Password `test`. Save. **Expected:** the card appears in the grid, with an "S" badge.
- [ ] **A2.2** Double-click the card. **Expected:** a new tab opens named `pw`; an overlay says "Connecting…", then "First time connecting to pw" with a fingerprint.
- [ ] **A2.3** Check the fingerprint against the server (`scripts/test-servers.sh status`, the line for server `a`, ED25519). **Expected:** they match.
- [ ] **A2.4** Press "Trust and connect". **Expected:** the overlay goes away and a shell prompt appears; the tab's dot turns to a connected state. The first connect also takes up to ~3 s extra (OS detection).
- [ ] **A2.5** Type `whoami`, `echo $TERM`, `seq 1 100000`, then `ls --color`. **Expected:** `test`, `xterm-256color`; the numbers scroll fast and the terminal stays responsive; colours show.
- [ ] **A2.6** Resize the window, then run `stty size`. **Expected:** it matches the visible rows and columns.
- [ ] **A2.7** Go back to the Vault tab: the host card for `pw` now has a letter badge (OS detection) and "last connected" is set (open ⌘T: `pw` is listed).
- [ ] **A2.8** Close the tab (⌘W). **Expected:** a confirmation appears because the shell is live; confirming closes it.

### A3. Connection drops and Reconnect (F11)

- [ ] **A3.1** Open `pw` again (no host-key question this time). In the shell, run `echo before`.
- [ ] **A3.2** In a Terminal: `docker restart dalan-test-ssh`. **Expected:** within about a minute the overlay "Connection closed" appears with the time and a Reconnect button (Enter does the same).
- [ ] **A3.3** Wait ~15 s for the server to be up, then press Reconnect. **Expected:** the same tab gets a new shell. The old scrollback is still there above.
- [ ] **A3.4** Run `scripts/test-servers.sh down` (all servers off), then try Reconnect in a live tab or open `pw`. **Expected:** "Can't reach 127.0.0.1:2222" with the hint about checking address/VPN/firewall. Then `scripts/test-servers.sh up` again (the host key stays the same).

### A4. Host key changed (F12, S18/S14)

Run these in a Terminal when asked.

- [ ] **A4.1** Make a new host key: `scripts/test-servers.sh rekey a new`. Wait for it to finish, then open `pw`. **Expected:** a blocking red warning "This server's identity changed" with the old and new fingerprints; the default (focused) button is Close/Cancel. It must **not** let you continue by pressing Enter.
- [ ] **A4.2** Press Close. **Expected:** the tab does not connect. Open **Known hosts** (left sidebar): the saved key for `127.0.0.1:2222` is still the old one.
- [ ] **A4.3** Open `pw` again, press "Replace saved key…". **Expected:** a confirmation that asks you to type the host name; the confirm button stays disabled until you type it correctly.
- [ ] **A4.4** Type the name, confirm. **Expected:** it connects; Known hosts shows the new fingerprint.
- [ ] **A4.5 Key-type switch.** Trust is now on the new key. Run `scripts/test-servers.sh rekey a only-ecdsa`, then open `pw`. **Expected:** the same blocking "identity changed" warning (the server now presents an ECDSA key where an Ed25519 one was saved; `status` shows it). **The audit says this will instead show the normal "first time" prompt; please say which you see, and do NOT click Trust if it is the first-time prompt.**
- [ ] **A4.6** Put it back: `scripts/test-servers.sh rekey a restore`, then if needed replace the saved key again (A4.3) or delete the row in Known hosts and trust the key.

**Stop here if you are short on time and send the report so far.**

## Part B. Connection states, keys and organising (about 20 minutes)

### B1. S18 connection states

- [ ] **B1.1 Password prompt.** Add host `ask` (127.0.0.1:2222, user `test`, no password). Connect, trust. **Expected:** a password prompt with a "Remember" checkbox. Cancel it: the tab closes or shows a cancelled state. Connect again, enter `test` with Remember on: it signs in. Close and open `ask` again: no prompt now.
- [ ] **B1.2 Auth failed.** Add host `wrong` (127.0.0.1:2222, user `test`, password `nope`). Connect. **Expected:** "Sign-in failed" with a reason and the buttons Edit host and Try again. "Edit host" opens the editor for that host.
- [ ] **B1.3 Unreachable.** Add host `dead` (127.0.0.1:2299, user `test`). Connect. **Expected:** "Can't reach 127.0.0.1:2299" with a Retry button, within a few seconds.
- [ ] **B1.4 Connecting + Cancel.** Add host `slow` with address `10.255.255.1`. Connect. **Expected:** the "Connecting…" overlay; its Cancel closes the tab. (It would time out after ~10 s otherwise.)
- [ ] **B1.5 KEX failure (optional).** `scripts/test-servers.sh kex-break c`, then connect to `inner` via bastion (see B4) or skip. If it connects anyway, note "KEX not triggerable" and move on. Put it back with `scripts/test-servers.sh kex-restore c`.
- [ ] **B1.6 Passphrase prompt.** Do B2.2 first, then come back: a host using the passphrase key, with the passphrase not saved, shows a passphrase prompt on connect.

### B2. Keychain (F02, S05-S08)

- [ ] **B2.1 Generate, install, connect (F02).** Keychain, Generate key (Ed25519, label `smoke`, no passphrase). Open its menu, "Install on a server…", pick host `pw`. **Expected:** a dialog; it asks about the host key and password if needed, then says the key was installed. Edit host `pw`: remove the saved password, set Key = `smoke`, save. Connect. **Expected:** you get a shell without any password question.
- [ ] **B2.2 Import keys.** Import key from the file `.test-servers/id_ed25519_passphrase` (use "Choose file…"). **Expected:** it asks for the passphrase (`passphrase`); wrong passphrase gives an error. Import `.test-servers/id_ed25519` too.
- [ ] **B2.3 Key server.** Add host `keyhost` (127.0.0.1:2223, user `keyuser`), Key = `id_ed25519`, no password. Connect, trust. **Expected:** shell, no password question. Then try the passphrase key: **Expected:** passphrase prompt (with Remember).
- [ ] **B2.4 Rejected key.** Make a second key `other` (Generate), set it on `keyhost`, connect. **Expected:** a message that the server rejected the key "other" (no password question, because none is saved).

### B3. Quick connect and groups (F03, F04)

- [ ] **B3.1 F03.** Press ⌘K, type `pw`, Enter. **Expected:** it opens (or focuses) a tab for `pw` in one step. Also check the ⌘K list shows hosts, tabs and actions.
- [ ] **B3.2 F04.** New host menu, New group `Production` with Username `test`, Password `test`. Inside it, New group `Web` with Port `2222`. Add host `grouped` (Address 127.0.0.1) with Group = `Web` and **no** username, password or port. **Expected:** the editor shows hints like "from Production". Connect: it signs in using the group's values.
- [ ] **B3.3** Drag host `ask` onto the `Production` group card. **Expected:** it moves; the breadcrumb works. Delete the middle group `Production`: **Expected:** a warning; its children move up one level.

### B4. Jump hosts (F05)

- [ ] **B4.1** Add host `inner`: Address `inner`, Port `2222`, Username `test`, Password `test`, Jump hosts = Custom, one hop = `pw`. Connect. **Expected:** the overlay says "via pw"; a host-key question for the bastion (if not already trusted), then one for `inner`; then a shell. Run `echo $HOSTNAME` and compare with `pw`'s: they differ.
- [ ] **B4.2** Try to delete `pw`. **Expected:** it refuses, naming `inner` as using it.

## Part C. Tunnels, files, snippets, workspaces, import (about 25 minutes)

### C1. Port forwarding (F06, S09/S10)

- [ ] **C1.1** Port forwarding, New rule: Local, server `pw`, listen `127.0.0.1:2230`, destination `inner:2222`. Save, switch it on. **Expected:** the state goes Starting then Active.
- [ ] **C1.2** In a Terminal: `nc 127.0.0.1 2230`. **Expected:** you see an `SSH-2.0-OpenSSH…` banner (press Ctrl-C). The rule card shows a connection count.
- [ ] **C1.3** SOCKS rule on `127.0.0.1:1080`, switch on. In a Terminal: `curl -sS --socks5-hostname 127.0.0.1:1080 telnet://inner:2222 -m 3`. **Expected:** an SSH banner appears.
- [ ] **C1.4** Port in use: in a Terminal run `nc -l 2999`; make a Local rule listening on `127.0.0.1:2999`, switch on. **Expected:** a clear message that the port is already in use. (Ctrl-C the `nc` afterwards.)
- [ ] **C1.5** `docker restart dalan-test-ssh` with a rule active. **Expected:** it shows Reconnecting, then Active again within about a minute.

### C2. SFTP (F07, S21/S22)

- [ ] **C2.1** Open the SFTP tab. Right pane: choose server `pw`. **Expected:** the home folder lists. Left pane shows your computer.
- [ ] **C2.2** Make a file: `echo hello > /tmp/smoke.txt`. Drag it from the left pane (go to `/tmp`) to the right pane. **Expected:** a transfer row with progress, then done; the file shows on the right.
- [ ] **C2.3** Drag the same file again. **Expected:** a conflict row (Replace / Keep both / Skip). Choose Keep both: `smoke (copy).txt` appears.
- [ ] **C2.4** Right-click a remote file: Rename, Permissions (set 600), Delete (with confirmation), New folder. **Expected:** each works.
- [ ] **C2.5** Big file: `dd if=/dev/zero of=/tmp/big bs=1m count=300`; drag it over and press Cancel mid-way. **Expected:** the transfer stops and the partial remote file is removed.
- [ ] **C2.6** Edit in place: right-click `smoke.txt` on the right: "Edit in default app". Change it in the editor and save. **Expected:** a toast "smoke.txt was saved" with Upload; pressing Upload says Uploaded; check with `cat` in a shell on `pw`.
- [ ] **C2.7** Try a folder you can't read (for example `/root` on the right pane). **Expected:** note what message you get (a raw error is a known gap).

### C3. Snippets (F08, S11-S13) and workspaces (F09, S19)

- [ ] **C3.1** Snippets: New snippet `who` with script `echo "hello from $HOSTNAME"`. Save.
- [ ] **C3.2** Run on hosts: pick `pw`, `keyhost`, `inner`. **Expected:** a workspace tab with three panes, each running the script once its shell is ready; `wrong` (if you add it) shows a red failure in its own pane.
- [ ] **C3.3** Workspace: press ⌘B, type `echo hi` in one pane. **Expected:** it appears in every connected pane; the toolbar shows how many are live. Press ⌘B again to stop. Try ⌘⇧M (split/focus), ⌘D (adds a pane).
- [ ] **C3.4** ⌘S, name it `smoke-ws`, save. Close the tab. Open ⌘T: **Expected:** `smoke-ws` is listed; opening it reconnects all panes.
- [ ] **C3.5** New Tab: tick three hosts, press "Open 3 as workspace". **Expected:** it opens with three panes.

### C4. Import (F10, S25)

- [ ] **C4.1** Hosts, Import, SSH config, Choose file…: `replica/smoke/ssh_config_sample`. **Expected:** a review list with `smoke-password`, `smoke-key`, `smoke-inner` (shows "via smoke-password") and `smoke-proxycmd` (a warning that ProxyCommand was ignored); a note that `Include` wasn't followed; wildcard `Host *` is not a row; User/Port come from `Host *`. Import them. **Expected:** the hosts appear, `smoke-inner` has the jump chain.
- [ ] **C4.2** Import the same file again. **Expected:** every row says "Already added" and is unticked.
- [ ] **C4.3** Import, CSV file: `replica/smoke/hosts_sample.csv`. **Expected:** a group `Smoke group` is created with two hosts in it; `csv-no-group` goes to the top level.

## Part D. Optional spot checks of later features (about 20 minutes)

These never ran against a real server. Any result is useful.

- [ ] **D1** ⌘T, Local terminal: a shell on your Mac; `ls`; `exit` closes the tab.
- [ ] **D2** In a session, press ⌘F, type part of some output: matches get selected, Enter steps through, Esc closes.
- [ ] **D3** Settings, Logs: turn on session logs and command history. Open `pw`, run `ls`, `echo hi`, close the tab. Logs shows a log; Replay shows the text. In a new session, ⌘. then History lists your commands.
- [ ] **D4** Edit host `pw`: Environment variables `LANG=C`; connect and `echo $LANG` (the Docker server accepts LANG). Terminal theme: choose Ember; the open tab changes.
- [ ] **D5** Settings, Shortcuts: change "New tab" to ⌘Y; ⌘T stops working, ⌘Y works; Reset.
- [ ] **D6** Telnet / serial / Mosh / AI / proxy / agent forwarding: skip unless you have the tools; if you want, see their notes in `replica/progress.md`.

## Tear down

```bash
scripts/test-servers.sh down
```

## Report back

Paste a list like this (only failures and surprises are needed):

```
A2.4 FAIL: <what you saw instead>
B1.2 OK but the Edit host button did nothing
C2.7 note: raw error "No such file" shown
Not run: B1.5, D6
```

Also paste anything from the red error toasts or the terminal where `pnpm tauri dev` is running.
