# Field report: a hands-on look at the reference app (2026-11-06)

For: Claude Cowork, to turn into improvement documents for Dalan.
Written by Claude Code after clicking through the reference SSH client that was already open on Mehrdad's Mac.

**Clean-room note.** This file lives in `replica/`, the only place the original may be named. It records *behaviour and structure only*, in our own words. It copies no UI text, icons, illustrations or theme names. Any improvement document made from it must be written in Dalan's voice (`replica/brand.md`) and use Dalan's tokens. Real host names, addresses and the account's email were seen on screen and are left out on purpose.

## 1. How the look was done, and what it could not cover

- **Access:** screen control at "click only" level (the app is a terminal-type app): left-clicks and scrolling, no typing, no key presses, no right-click, no drag.
- **Safe by design:** nothing was connected to, saved, deleted or changed. Drafts that were opened (new host, new forwarding, new snippet) were discarded. A local terminal tab and a New Tab were opened and closed again. The app was left on its two original tabs.
- **Version:** an installed desktop build, free ("starter") plan, signed in to a cloud account with sync on, 3 saved hosts, no keys, no snippets, no forwarding rules.
- **Not seen (needs a hand check):** a real SSH connection (host-key prompt, password prompt, error and reconnect states, the connected terminal's own chrome), SFTP against a server and transfers, workspaces and split panes, broadcast, the Serial flow, key generation and import, snippet running, the AI features if any, right-click menus, drag and drop, keyboard flows, anything gated by the paid plan.

## 2. Inventory of what exists (by screen)

### Window and navigation
- One window with a **top tab bar**: a pinned "Vaults" tab (with a sync-cloud glyph and a vault switcher dropdown), a pinned "SFTP" tab, then session tabs, and a "+" to add one. Right side: an "Update" pill and a notifications bell (opens a small popover that loads from the server).
- Inside the Vaults tab a **left list** with six sections: Hosts, Keychain, Port Forwarding, Snippets, Known Hosts, Logs.
- The **menu bar** lists Vaults ⌘1, SFTP ⌘2, Settings ⌘, and has View items for the terminal side panel and terminal font size.
- **Vault switcher:** a personal vault (synced) and a team vault (people icon). Everything in the sections is per vault.

### Hosts
- A single search field that doubles as **quick connect** ("find a host or type user@host") with a Connect button.
- Toolbar: New host (split button), **Terminal** (opens a local shell), **Serial**, then view, tag filter and sort icons, then a members avatar with a "+" (team sharing).
- Hosts are **cards** with an OS-detected icon and a one-line summary (protocol and tags). Grid or list view. Sort by name A-Z / Z-A or by date newest / oldest. A tag icon opens a tag filter popover (empty state explains tags).
- New host menu: New Group, Import, and three **cloud-provider imports** (greyed with a lock: paid).
- **New host is a right-hand panel, not a modal.** Address comes first (host or IP); a *live draft card* appears in the grid as you type. Fields: vault, label, parent group, tags, a "backspace key" behaviour setting, "share this host", SSH port written as a sentence ("SSH on [22] port"), credentials (username, password with show toggle, a link to attach an SSH ID, key, certificate or FIDO2 key).
- Under credentials, a stack of **collapsed option rows** that each open a sub-panel with a back arrow: agent forwarding (off), startup snippet, host chaining (a small visual chain with an "add a host" button and a clear button), proxy (type, host, port with a default, user, password), environment variables, character encoding, Mosh (enabled/disabled dropdown), a per-host terminal theme preview row, and an "add Telnet" link (Telnet is a second protocol block on the same host, not a separate host type).
- A big **Connect** button at the bottom of the panel; closing a half-filled draft asks "discard changes?" with a single destructive button.

### Keychain
- Empty state plus toolbar: New key (with a menu), **Certificate**, **Touch ID**, **FIDO2**. Search, view and sort icons.

### Port forwarding
- New forwarding opens a **wizard in the side panel**: step 1 picks Local / Remote / Dynamic as a segmented control with an *illustrated diagram* and a plain-language sentence; Continue or **Skip wizard**; step 2 asks for the local port and bind address (the placeholder bind address is all interfaces, which is the unsafe default Dalan avoids). Back link at each step. A draft rule card shows in the list while editing.

### Snippets
- New snippet panel: an "action description" field with a small **AI-assist** glyph, package, script, share, and a **Targets for execution** block ("add targets") with a Run button inside the editor. A **Shell History** shortcut button in the toolbar.

### Known hosts
- Tiles of trusted addresses, an **Import** button. Search, view and sort icons.

### Logs
- A **session history table**: date range and duration, the account and the machine used, the host, and a "saved" bookmark column. On the free plan the top says logs are not available and offers an upgrade, yet the connection history rows still list.

### SFTP tab
- **Two panes.** Left is the local computer, shown immediately; right says "connect to host" with a Select host button. Columns: name, date modified, size, kind. Each row shows its permissions under the name. Back / forward arrows, a breadcrumb, a Filter box and an **Actions** menu.

### Terminal tab (local terminal opened to look)
- A tab with a close X and a small settings glyph; the terminal fills the window edge to edge with no padding or header.
- A **side panel** toggled from the top right (also in the View menu) with four tabs: (1) terminal search and an *Autocomplete (beta)* switch, (2) snippets, (3) shell history with a nudge to save the top 3 commands as snippets (Close / Save), (4) themes with a font entry and a usage count per theme (community popularity).
- **New Tab** screen: a search field (⌘K) and a **Recent connections** list (host, vault name) with "Create a workspace" and "Restore" buttons (select several to make a workspace; restore a closed session set).

### Settings (its own window)
- Sections: Account (plan, 2FA toggle, change email, log out, change encryption password, delete account, **sync of keys and identities** with last-sync time and Sync now), SSH ID, Invite People, Terminal, SFTP, Logs, Shortcuts, a Theme (Auto/dark/light) control, and links to docs, request a feature, report a bug, a beta programme and a changelog.
- **Terminal:** autocomplete (beta), autoreconnect (on), import shell history (on), select-to-copy with right-click paste (off), bell sound, Option as Meta, bright colours for bold, terminal emulation type, font family and size.
- **SFTP:** remembered "open with" apps per file type (empty list explains how entries get added).
- **Logs:** retention per vault, delete existing logs, plan-gated.
- **Shortcuts:** a platform picker and a **read-only list** of about 20 hotkeys (tabs 1-9, move focus between workspace terminals, copy/paste/select all, open local terminal, open serial, close tab, next/previous tab, port forwarding, command palette, jump to, new tab, terminal search, show side panel, broadcast mode, toggle workspace view, save workspace template, snippets). Two actions show "Disabled".
- **SSH ID:** an explainer and a set-up button for passkey-style SSH sign-in tied to an account.

## 3. Patterns worth learning from (ideas, not copy)

1. **Side panels instead of modals** for create/edit, with a **live draft card** in the list. It keeps context visible and makes a half-finished item obvious.
2. **Drill-in sub-panels** with a back arrow for advanced options (chain, proxy, env, encoding), so the first screen stays short. Dalan's host editor is one long scroll.
3. **Address-first host creation** and a search box that doubles as quick connect with user@host.
4. **Wizards that explain with a picture** (port forwarding) plus an obvious "skip".
5. **Quick-reference empty states** on every section that say what the thing is for in one line.
6. **Recent connections and "restore"** on the new tab screen; "create a workspace" from a multi-select there.
7. **Terminal side panel** that gathers search, snippets, history and theme in one place, with history turned into snippet suggestions.
8. **Community theme counts** as a way to choose a theme.
9. **Per-section toolbars** with the same four controls everywhere (search, grid/list, sort), which makes sections feel alike.
10. **SFTP rows that show permissions inline** and an Actions menu.

## 4. Where Dalan already matches or is ahead (from this look)

- Host-key verification and changed-key blocking (the reference was not tested for this).
- Safe tunnel defaults: bind to 127.0.0.1 and a warning when exposed.
- No account needed; encrypted local vault; history and logs encrypted and not paywalled.
- Command palette, tab dots with a real meaning, shortcut labels that follow rebinding.
- Rebindable shortcuts (the reference list is read-only).
- Local-only data; the reference signs in, syncs and gates features by plan.

## 5. Gaps Dalan has against what was seen

Ordered by likely value. Effort: S (days or less), M (about a week), L (weeks).

| # | Gap | Seen in the reference | Effort | Notes for the improvement doc |
|---|-----|-----------------------|--------|-------------------------------|
| 1 | Host editor as a side panel with a live draft card and drill-in sub-panels | New host, forwarding, snippet | M | Dalan already has a sheet; the work is restructuring, not new backend. |
| 2 | Quick connect from the search field (`user@host[:port]`) | Hosts | S | Palette partly does this; add parse and connect-without-saving, with an offer to save. |
| 3 | Local terminal and Serial as first-class toolbar buttons | Hosts toolbar | S | Both exist in Dalan; surfacing them is a UI change. |
| 4 | Sort and tag-filter controls on every section | Toolbars | S | Hosts only today. |
| 5 | Recent connections with "restore session set" | New Tab | S-M | Recents exist (P17); restore of the last open tabs on launch is new. |
| 6 | Terminal side panel: terminal search, themes with preview, font | Side panel | M | Dalan has snippets and history; add search/theme/font tabs. |
| 7 | History-to-snippet suggestions | Side panel | S | Needs History (exists) plus a "save as snippet" action. |
| 8 | Wizard illustrations for tunnels | Forwarding | S | Needs our own drawings, not copies. |
| 9 | Autocomplete (shell-aware) | Settings, side panel | L | Marked beta in the reference; was left in Dalan's backlog. |
| 10 | Shell-history import, autoreconnect and bell options as visible settings | Terminal settings | S | Autoreconnect exists as a behaviour; expose a switch. |
| 11 | "Open with" app list for SFTP | SFTP settings | M | The one item left out of P18. |
| 12 | Hardware keys: Touch ID and FIDO2 | Keychain | L | Backlog already; clarify scope before building. |
| 13 | Teams, sharing, cloud sync | Vault switcher, Invite People, sync | L (and a decision) | Conflicts with "no account"; see open questions. |
| 14 | Cloud-provider host import (AWS, DigitalOcean, Azure) | New host menu | M each | Needs API keys handled in Rust only. |
| 15 | Host-level Telnet block on the same host | New host panel | S | Dalan models protocol per host. Decide whether to merge. |
| 16 | A visual host-chain editor | Host chaining | S-M | Dalan has the chain; the picture is the gain. |
| 17 | Community theme usage counts | Themes | n/a | Needs a server; skip. |

## 6. Things in the reference that Dalan should deliberately not copy

- Plan gating of logs and basic features (Dalan's promise is the opposite).
- A tunnel default that listens on every network interface.
- Required sign-in for a desktop SSH client.
- An "AI" helper on the snippet label (only if it can run without sending commands to a third party; Dalan already has an opt-in, own-key version).

## 7. Open questions to settle with Mehrdad before writing the documents

1. Is **team sharing / sync** ever in scope? (Dalan is local-first; "self-hosted sync" is in the backlog.) This decides rows 13 and parts of 5.
2. Should Dalan move to **side-panel editors** everywhere, or keep sheets and just add drill-in sections?
3. How far to go on **autocomplete**: a simple history/snippet completion, or shell integration?
4. **Hardware keys** (Touch ID, FIDO2): wanted for the team, or later?
5. Which of the **cloud imports** matter for his team?

## 8. Suggested checks to do by hand next (they could not be done here)

Run these against throwaway servers (`scripts/test-servers.sh up`), never real ones, then add the findings here:
1. First connection: what the host-key question looks like, what each error looks like (wrong password, unreachable, changed key), and how reconnect behaves.
2. The connected terminal's tab chrome, split panes, broadcast, and how a workspace looks.
3. SFTP against a server: transfers, conflicts, drag and drop, context menu, editing in place.
4. Key generation, import and "install on server".
5. Keyboard flows: tab switching, jump to, the palette, and what happens with the shortcuts inside a terminal.
6. Performance feel: startup time, typing latency, large output.

## 9. What Cowork should produce from this

- A **gap-closing plan** for Dalan (rows 1-12 first), each item with the screen IDs it touches (S02, S03, S09, S10, S16, S20, S21, S23), user value, risk and effort.
- A **UI-pattern note** for the design system (`replica/design/components.md`): side-panel editor, draft card, drill-in sub-panel, wizard step with picture.
- A short **"don't copy" list** (section 6) to keep the clean-room line clear.
- A list of **hand checks** (section 8) to finish the picture before deciding on the larger items.

## 10. Part 2: observed against the local Docker test servers

Added the same day. The test servers (`scripts/test-servers.sh`) were brought in by importing a temporary `~/.ssh/config` through the reference app's Import dialog (clicks only; no typing is possible there). Only the three test hosts and the one throwaway test key were ticked; the user's real keys and known-hosts entries were left unticked. The temporary config and key file were deleted from `~/.ssh` afterwards.

### Import from ~/.ssh
- The dialog offers: ~/.ssh, CSV, SecureCRT, PuTTY, MobaXterm, and says it carries connections, SSH keys, known hosts and port forwarding.
- It **reads `~/.ssh` immediately** (no file picker) and lists **Hosts** (config entries *and* every known_hosts entry, 15 in this case) and **Keys** (every private and public key file it finds, 4 to 5) in collapsible groups with a count and a tick per group and per row.
- Everything is ticked by default, including real private keys. There is a filter box, Later, Back and Import. Import happens with no further confirmation and the new hosts appear straight away.
- **Compare with Dalan:** Dalan's import shows a review list, unticks duplicates and notes ignored lines, but it never touches keys or known_hosts. The reference's "everything ticked, private keys included" default is a risk worth not copying; its group tick-boxes (select or clear a whole group) are a useful idea.

### Connection screen (every connect)
- A dedicated full-tab screen: host name with its address, a **Show logs / Copy logs** button, and a **three-stage progress track** (start, secure channel, shell) whose connectors fill as it advances and turn red on failure.
- **Unreachable host:** the track turns red, the heading says the connection failed, and a **step-by-step connection log** is shown (starting connection, address resolution, connecting, then the failure reason with "no more addresses to try"), with a link to documentation. Buttons: **Close, Edit host, Start over.**
- **First connection / unknown key:** the track stops at the middle stage. It states that the authenticity can't be established, shows the key type and fingerprint in plain text, and asks "add it to the list of known hosts?". Three buttons: **Close**, **Continue** (this once) and **Add and continue** (primary). So "trust once" exists and is a first-class choice.
- **Key login:** with an imported key and no password stored, it signed in silently and went straight to the shell. No overlay, no toast.
- After connecting, the tab shows the host name with a close X (no status dot or colour was visible), the terminal fills the tab, and the **side panel reopened on the tab it was last used on** (themes).
- A connected host's **card in the vault shows a live badge** instead of its OS icon, and the host list reorders with recently used first.
- **Not captured:** a changed-key screen (needs a rekeyed server, not done), a password or passphrase prompt, wrong-password error, auto-reconnect after a drop.

### SFTP against a server
- The SFTP tab is two panes. The right pane's **Select host** is a searchable picker (vault selector, host list with the live badge, a Local shortcut button and sort control). It needs a **double-click** to open a host; a single click only highlights.
- Both panes then have the same header: place name, Filter, an **Actions** menu, back/forward and a breadcrumb; columns for name (with permissions printed under each name), date modified, size and kind. Folders sort first; sort is by the date column by default.
- **Actions menu (this is the context menu's equivalent, reachable without right-click):** Open, Open with, Copy to target directory, Rename, Delete, Refresh, New Folder, Show Hidden Files, Edit Permissions, Select All, Close. Items that need a selection are greyed until a row is selected.
- **Copy to target directory** copied the selected remote file into the local pane's folder immediately, with no confirmation dialog. **No transfer progress panel was visible** for a 4-byte file; whether a queue or a toast appears for bigger files was not seen. The local pane did not visibly refresh to show the new file in the screenshot taken straight after.
- The test file written to the home folder was removed afterwards.

### Workspaces and split panes
- **Not reached.** Making a workspace needs a multi-select on the New Tab screen, which in this tier would need a modifier-click, and recent connections there only listed two older real hosts, not the two live test sessions. The File menu only offers New Local Terminal and New Serial Connection. The Window menu could not be opened because a Claude Quick Entry bar took the input. A hand check is still needed.

### What is still not seen
Right-click menus, drag and drop, typing-based flows (key generation, snippet running), workspaces/splits, broadcast, a changed host key, password prompts and reconnect after a drop. The click-only access level makes these impossible for an automated look; they need Mehrdad's hands or a screen recording.

### New ideas from part 2 (add to section 5 of the gap table)
| # | Gap | Effort | Notes |
|---|-----|--------|-------|
| 18 | A connection screen with a 3-stage progress track and a readable connection log with "copy logs" | S-M | Dalan's overlay has states but no live step log; a log helps support and debugging. |
| 19 | "Continue once" next to "Add and continue" on the first-connection question | S | Dalan has trust-once in Rust (remember false) but no button for it yet. |
| 20 | Group tick-boxes and a filter in the import review | S | Keep Dalan's no-keys-by-default behaviour. |
| 21 | Reopening the side panel on its last tab | S | |
| 22 | A live badge on host cards for connected hosts | S | Dalan shows the dot on tabs only. |
| 23 | An Actions menu on each SFTP pane as the keyboard- and click-reachable twin of the context menu | S | Dalan has right-click and F2 but no visible button. |

## 11. Part 3: events triggered from the shell while watching

The shell was used to cause server-side events (write to the session's terminal, restart the container, rekey it) while the reference app was on screen. Typing into the app is still impossible; this is the way around it.

### Terminal behaviour
- Output written by the server appeared at once, with 16 ANSI colours and bold, reverse and underline rendered correctly. The terminal fills the tab, no padding.
- **Background activity:** a session tab that received output while another tab was in front got a small **green dot on its icon** (unread output), the same idea as Dalan's tab dots.

### Connection drop and automatic reconnect
- When the server restarted, a **small card appeared at the bottom-left** of the window: it counted the dropped sessions ("2 sessions are disconnected"), said it was initiating reconnection, with a spinner, and offered **Reconnect (Enter)** and **Close terminals**. The tab icons showed a spinner.
- **About six seconds later both sessions were back in the same tabs** and the old scrollback was kept above a new login banner. No click was needed (an "autoreconnect" setting exists and is on).
- One card covers all dropped sessions at once rather than one overlay per tab.

### Changed host key
- After the server was rekeyed and restarted, the auto-reconnect stopped at the key check and showed a full-tab screen: a **red heading that the fingerprint for the address has changed**, a line about a possible security breach, and **only the new fingerprint** (the old one was not shown). The progress track stopped at the middle stage.
- Buttons: **Replace** (primary blue, first), **Add as new** (secondary) and **Cancel** (red, right). The dangerous choice is the most prominent and no name has to be typed to confirm.
- A second tab to the same host showed the same screen (one per tab), and a small orange dot marked the waiting tab.
- **Compare with Dalan:** Dalan shows both fingerprints, makes Close the default and requires typing the host name to replace. That is stronger and should stay; the worthwhile idea is only the single combined card for several dropped sessions (row 24 below).

### Host and key management
- **Host Details panel** (opened from a pencil on the card): vault, address, label, group, tags, port, credentials. It showed a **suggestion that two hosts reuse the same credentials, with a link to merge them into an identity**. Menu: Connect, Add Telnet, Duplicate, **Remove** (a confirmation names the host being removed).
- **Edit Key panel shows the full private key text in the clear**, with the public key and certificate boxes and an "Export to host" button; menu: Export to host, Collaborate, Remove (confirmation says "delete 1 key"). Showing key material in the interface contradicts Dalan's rule that secrets never reach the UI. Do not copy.
- **OS detection:** after a first connect the host card's icon changed from a generic server to the detected OS (Alpine for the test containers).
- **Known Hosts:** entries are tiles with no visible delete (no hover button; selection only highlights). Removal presumably needs a right-click, which this look could not do. A single "Add and continue" earlier added `[127.0.0.1]:2223` here. **Import side effect:** importing from `~/.ssh` with the Hosts group unticked still added all the user's known_hosts entries (13 extra tiles) to this list, so "untick what you don't want" did not apply to known hosts.

### Gaps and ideas added
| # | Gap | Effort | Notes |
|---|-----|--------|-------|
| 24 | One bottom-left card for all dropped sessions, with Reconnect all and Close all | S-M | Dalan has an overlay per tab; a workspace of 10 panes would benefit. |
| 25 | A visible unread dot on background tabs is confirmed as the expected pattern | n/a | Already built (P18). |
| 26 | "These hosts reuse the same credentials: make an identity" suggestion | S-M | Dalan has identities (P8); the nudge is new. |
| 27 | Auto-reconnect that resumes in the same tab with scrollback kept | S | Dalan already reconnects on request and keeps scrollback; consider an opt-in automatic retry with backoff. |

### Still not seen
Workspaces and split panes (no way to multi-select from this access level), the password and passphrase prompts, wrong-password error, right-click menus, drag and drop, key generation and snippet running (all need typing, right-click or drag).

### Cleanup state after this look
Removed from the reference app: the three test hosts and the four imported copies of the throwaway key. **Left behind (could not be removed with clicks):** 13 added entries in Known Hosts (`[127.0.0.1]:2223` and 12 from the user's own known_hosts file); a "sessions" history row in Logs for the test connections. The temporary `~/.ssh/config` and `~/.ssh/dalan_test_key` were deleted; the test server's host keys were restored.

## 12. Part 4: guided session (the user did the typing, right-clicking and dragging; Claude captured)

The earlier "not seen" list is now covered, except where noted. Servers were the local Docker test servers.

### Password and authentication screens
- **After a failed sign-in** the connection screen shows a **six-stage progress track** (connect, host key, password, two keyboard-interactive stages, shell) that turns fully red, with the log listing the methods the server allows (public key, keyboard-interactive, password), "authenticating using password method", "authentication failed", and "no more authentication methods to try". Buttons: Close, Edit host, **Start over**. There is no inline "try again" field.
- **After "Start over"** the screen shows a **three-way method chooser: Password, Interactive, Public Key**, above a single password field with a show/hide eye, a Close button, and a **Continue button with a dropdown arrow**. The track is back to three stages. A correct password went straight to the shell.
- **Compare with Dalan:** Dalan keeps the password card in place and says which host rejected it. The method chooser (letting the user pick password, keyboard-interactive or key explicitly) is new and could help with servers that offer several methods. The log-as-explanation is worth borrowing (row 18).

### Key generation
- The **Generate Key** side panel: vault, label, key type as a segmented control, a passphrase field with eye and a **"save passphrase" switch (off by default)**, and **Generate & save**. A draft card with the type appears in the list while editing.
- **Type options and hints:** ED25519 (default, hint "OpenSSH 6.5+"); ECDSA (hint "5.7+", curve sizes 521, 384, 256); RSA (hint "legacy devices", sizes 4096, 2048 and **1024**); and **ML-DSA** (hint "only on ML-DSA-enabled servers", parameter sets 87, 65, 44), a post-quantum signature type.
- The saved key's panel shows the **private key in the clear**, the public key (with a generated comment naming the app) and a certificate box, plus **Export to host**.
- **Compare with Dalan:** Dalan offers Ed25519, ECDSA P-256/384/521 and RSA 4096/3072/2048. Not worth copying: 1024-bit RSA, showing the private key. Worth considering: an explanatory hint per type, and a clear note about which servers support a type.

### Key export (install on a server)
- **Key Export** panel: choose a host, a location (default `.ssh`), a file name (default `authorized_keys`), a note that only UNIX systems are supported, and an **Advanced** section with the **editable shell script** that does it (makes the folder, sets modes 700 and 600, appends a comment line and the key; the location, file name and key are passed as arguments). Button: Export and Attach.
- The username was asked inside the panel, with a small three-stage track.
- **Result seen on the server:** the key was appended **twice** (the script has no check for an existing copy), each time with a comment line. Dalan's installer should keep its duplicate check and strict path validation; a user-editable script is a risk Dalan avoids.

### Snippets
- The snippet editor **saves as you type** (a check mark at the top, a card appears at once). Fields: action description, package, script, share, **Targets** (list with an Edit link), and a **Run** button pinned to the bottom.
- **Running** opened a **new tab named after the snippet** with a **left list "Terminals · N"** (one row per target with an X) and the terminal beside it. The script was typed into the shell and ran as typed (visible, and in the history). Running the same snippet again opened **a second tab** rather than reusing the first.
- A button above the list switches the tab between that **list layout** and a **split layout** (framed panes).

### Workspaces and split panes
- **Dragging one session tab onto another merged them** into a workspace tab, with the two terminals **stacked vertically** (top and bottom), each in its own frame. Dragging the second tab onto the first needed no confirmation.
- **Pane header:** host name, protocol label, a small dot when a pane has unread output, a **Broadcast** toggle, maximise and close. The focused pane has a solid white frame.
- **Broadcast on:** the other pane(s) get a **dashed frame**; the toggle highlights. Off restores the frames.
- The **host card menu has "Add to Workspace"** with a list of existing workspaces and "New Workspace", which is the click-only way to build one.
- Layout choices seen: list plus one big terminal, or split frames. Resizable divider handles were not tested.

### Right-click menus
| Where | Entries |
|-------|---------|
| Tab in the tab bar | Rename, Close |
| Inside a terminal pane | none (no menu; copy and paste on right-click is a setting, off by default) |
| Pane header | none |
| Host card | Quick Connect (Return), Connect (submenu: with SSH, with SFTP, in a new window while holding Option), Add to Workspace (submenu), Edit Host Details (E), Collaborate, Move to (submenu), Copy to (submenu), Duplicate, Copy Link, Remove |
| Key card | Edit (E), Export to host, Collaborate, Move to, Copy to, Remove |
| SFTP file or folder | Open, Open with, Copy to target directory, Rename, Delete (red), Refresh, New Folder, Edit Permissions |
| SFTP blank space | the same list with the selection-only items greyed out; Refresh and New Folder active |

- Single-letter shortcuts (E, Return) are shown next to menu items. Menus use icons, a red destructive item, and submenu arrows. The tab menu is minimal.
- **Compare with Dalan:** Dalan has right-click menus on SFTP rows and tags but not on host cards, keys, tabs or panes. A host-card menu with Connect (SSH or SFTP), Add to Workspace, Edit, Duplicate, Copy to/Move to groups and Remove is the biggest gap here (new row 28), followed by a tab menu with Rename and Close (row 29).

### Drag and drop
- **Tab onto tab** creates a workspace (above).
- **File from the local pane onto the remote pane** transferred a 200 MB file to the server and the remote list refreshed to show it, all within a few seconds on localhost. **No progress bar, queue or toast was visible** in the screenshots taken after the drop; on a fast local link it may have been too quick to catch. Dropping onto a folder row was not captured.

### Updated gap table additions
| # | Gap | Effort | Notes |
|---|-----|--------|-------|
| 28 | Right-click menu on host cards (connect via SSH or SFTP, add to workspace, edit, duplicate, move, copy, remove) | M | Needs a context-menu component plus store actions; most already exist. |
| 29 | Tab menu with Rename and Close, and a split of "add to workspace" on a host | S | Rename exists (double-click); adding the menu is cheap. |
| 30 | Auth method chooser (password, interactive, key) after a failed attempt | S-M | Keyboard-interactive was added in P17; the choice is currently automatic. |
| 31 | Per-type hints in key generation ("works with OpenSSH 6.5 and newer") | S | Copy only. |
| 32 | A snippet run that opens a list-plus-terminal tab with per-target rows | M | Dalan opens a workspace of panes; consider the list layout as an alternative. |
| 33 | Workspace made by dragging a tab onto a tab | Done | Added in P18 (opens a workspace with fresh connections). |
| 34 | A dashed frame on panes that receive broadcast input | S | Dalan shows a "Broadcasting to N" pill; the per-pane frame is a clearer cue. |

### What is still not covered
Transfer progress and conflict handling during a slow or large transfer (localhost was too fast), resizable splits, drop onto a folder row, terminal-side autocomplete, and anything gated by a paid plan.

### Cleanup done after part 4
Removed from the reference app by clicking (each removal confirmed by its own dialog naming the item): the two test hosts (the original and its copy), the generated key, both snippets (the original and its copy), and the workspace tabs. On the test server: the 200 MB file, the added `authorized_keys` lines (the file is back to its earlier two lines), and the local 200 MB file. **Still left in Known Hosts (no delete control reachable by clicking):** 14 added entries now, `[127.0.0.1]:2222`, `[127.0.0.1]:2223` and 12 imported from the user's own known_hosts file. A few history rows in Logs for the test sessions also remain.
