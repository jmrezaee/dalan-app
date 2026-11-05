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
