# Field report: a hands-on look at the reference app (2026-11-06)

Rewritten on 2026-11-06 (P26) for the clean room. The first version quoted the reference app's labels, hints and menu entries. This one describes behaviour and structure only, in our own words. It quotes no UI text and names no theme. Real host names, addresses and the account's email are left out.

The reference app is named here because this file lives in `replica/`. Any improvement document made from it must be written in Dalan's voice (`replica/brand.md`) and use Dalan's tokens.

## 1. How the look was done, and what it could not cover

- **Access:** screen control at click-only level at first (no typing, no right-click, no drag). Later parts had Mehrdad do the typing, right-clicking and dragging while Claude captured.
- **Safety:** only local throwaway Docker servers were connected to. Drafts were discarded.
- **Version:** an installed desktop build on the free plan, signed in to a cloud account with sync on.
- **Not covered:** the paid plan's features, autocomplete, resizable splits, drop onto a folder row, and transfer progress on a slow link (localhost was too fast to see it).

## 2. Inventory of what exists (by screen)

### Window and navigation
- A top tab bar: a pinned vault tab with a sync indicator and a vault switcher, a pinned SFTP tab, session tabs and an add button. On the right: an update pill and a notifications bell.
- The vault tab has a left list with six sections: hosts, keychain, port forwarding, snippets, known hosts, logs.
- The menu bar offers the vault and SFTP tabs, settings, the terminal side panel and font size.
- A personal vault (synced) and a team vault. Everything in the sections belongs to one vault.

### Hosts
- One search field that is also quick connect for a `user@host[:port]` text.
- Toolbar: new host, local terminal, serial, view toggle, tag filter, sort, and team sharing.
- Hosts are cards with a detected-OS icon and a short summary. Grid or list. Sort by name or date.
- Cloud-provider imports exist but are locked behind a paid plan.
- **Editing a host happens in a right-hand panel, not a modal.** Address comes first, and a draft card shows in the grid while typing. Fields: vault, label, parent group, tags, backspace behaviour, sharing, port, and credentials (username, password, or an attached identity, key, certificate or hardware key).
- Under credentials are collapsed option rows, each opening a sub-panel with a back arrow: agent forwarding, startup snippet, host chain (drawn as a small chain), proxy, environment variables, character encoding, Mosh, a per-host terminal theme, and a second protocol block (Telnet) on the same host.
- Closing a half-filled draft asks whether to discard it.

### Keychain
- Toolbar with new key, certificate, Touch ID and hardware-key entries, plus search, view and sort.

### Port forwarding
- Creating a rule is a short wizard in the side panel. Step 1 picks local, remote or dynamic, with a drawing and a plain sentence. There is a skip option. Step 2 asks for ports. The suggested bind address is every interface, which is unsafe; Dalan binds to 127.0.0.1.
- A draft rule card shows while editing.

### Snippets
- The editor has a description (with an AI-assist glyph), a package, the script, sharing and a list of run targets with a Run button inside the editor. A shell-history shortcut sits in the toolbar.

### Known hosts
- Tiles of trusted addresses, with an import button and search, view and sort.

### Logs
- A table of past sessions: date range, duration, account, machine, host and a saved column. The free plan hides log contents but still lists the rows.

### SFTP tab
- Two panes. The local side shows at once; the other side asks you to pick a host. Columns: name, modified, size, kind. Permissions are printed under each name. Back and forward arrows, a breadcrumb, a filter box and an actions menu.

### Terminal tab
- The terminal fills the tab edge to edge.
- A side panel toggled from the top right, with four tabs: search plus an autocomplete switch, snippets, shell history (with a nudge to save frequent commands as snippets), and themes with a font entry.
- A new-tab screen with a search field and a recent-connections list, with actions to make a workspace from several of them and to restore a closed set of sessions.

### Settings (its own window)
- Sections: account (plan, two-factor, sync of keys and identities), SSH ID, invite people, terminal, SFTP, logs, shortcuts and theme.
- Terminal options include autocomplete, automatic reconnect (on), shell-history import (on), copy on select, bell sound, meta key, bright bold colours, emulation type, font family and size.
- SFTP has a remembered list of apps per file type.
- Logs retention is per vault and plan-gated.
- Shortcuts are a read-only list of about 20 entries, two shown as disabled.

## 3. Patterns worth learning from (ideas, not copy)

1. Side panels instead of modals for create and edit, with a live draft card.
2. Drill-in sub-panels with a back arrow, so the first screen stays short.
3. Address-first host creation, and a search box that doubles as quick connect.
4. Wizards that explain with a picture, plus an obvious way to skip.
5. One-line empty states that say what each section is for.
6. Recent connections with restore, and a workspace made from several of them.
7. A terminal side panel that gathers search, snippets, history and theme, turning history into snippet suggestions.
8. The same four controls (search, view, sort, filter) on every section toolbar.
9. SFTP rows that show permissions inline, with an actions menu.

## 4. Where Dalan already matches or is ahead

- Host-key verification and changed-key blocking.
- Safe tunnel defaults.
- No account; an encrypted local vault; history and logs encrypted and not paywalled.
- Command palette, meaningful tab dots, shortcut labels that follow rebinding, and rebindable shortcuts.

## 5. Gaps found, and what happened to them

| # | Gap | Effort | Outcome |
|---|-----|--------|---------|
| 1 | Host editor with a draft card and drill-in pages | M | Built in P23 |
| 2 | Quick connect from the search field | S | P22 (fix due in P27) |
| 3 | Local terminal and serial as toolbar buttons | S | Local terminal in P22; serial not |
| 4 | Sort and filter on every section | S | P22 |
| 5 | Restore the last open tabs on launch | S-M | Not built |
| 6 | Side panel with theme and font tabs | M | P24 |
| 7 | History-to-snippet suggestions | S | P24 |
| 8 | Wizard drawings for tunnels | S | Needs our own drawings |
| 9 | Autocomplete | L | Backlog |
| 10 | Visible switches for reconnect and bell | S | P21, P25 |
| 11 | Remembered apps for SFTP | M | Built after P25 |
| 12 | Touch ID and hardware keys | L | Backlog |
| 13 | Teams, sharing, cloud sync | L, a decision | Conflicts with no-account; open |
| 14 | Cloud-provider host import | M each | Backlog |
| 15 | Second protocol block on the same host | S | Open decision |
| 16 | Visual host-chain editor | S-M | P23 |
| 17 | Community theme counts | n/a | Needs a server; skipped |
| 18 | Connection screen with a readable step log and copy | S-M | P21 |
| 19 | Connect-once beside trust-and-save | S | P21 |
| 20 | Group tick boxes and a filter in the import review | S | P25 |
| 21 | Reopen the side panel on its last tab | S | P24 |
| 22 | Live badge on connected host cards | S | P22 |
| 23 | Visible actions button on SFTP panes | S | P22 |
| 24 | One card for all dropped sessions | S-M | P21 |
| 26 | Suggest an identity when hosts share credentials | S-M | P23, to be reworked in P27 |
| 27 | Automatic reconnect with backoff | S | P21 |
| 28 | Right-click menu on host cards | M | P22 |
| 29 | Tab menu | S | P22 |
| 30 | Choose a sign-in method after a failure | S-M | P21 |
| 31 | Per-type hints when generating a key | S | P25 |
| 32 | Snippet run as a list-plus-terminal tab | M | Not built |
| 33 | Workspace by dragging a tab onto a tab | Done | P18 |
| 34 | Frame on panes that receive broadcast input | S | P24 |

## 6. Things Dalan should deliberately not copy

- Plan gating of logs and basic features.
- A tunnel default that listens on every network interface.
- Required sign-in for a desktop SSH client.
- An AI helper that sends commands to a third party without an opt-in with your own key.
- Showing a private key's text in the interface (hard rule 1).
- A key-install script the user can edit, and one that can add the same key twice.
- An import that ticks private keys and known-hosts entries by default, and that adds known-hosts entries even when their group is unticked.
- A host-key change screen where replacing is the most prominent choice and needs no typed confirmation.
- A 1024-bit RSA option.

## 7. Behaviour observed against the local test servers

- **Import from the SSH folder.** The dialog read the folder without a file picker. It listed hosts (config entries and every known-hosts entry) and every key file, all ticked by default, with a tick per group and per row, and a filter box.
- **Connection screen.** A full-tab screen with a progress track that fills as it advances and turns red on failure, a live step log with a copy action, and buttons to close, edit the host or start over. A first connection stops at the host-key stage showing the key type and fingerprint, and offers continuing once, or adding to known hosts and continuing.
- **After a failed sign-in.** The log listed the methods the server allows and the attempts made. Starting over showed a chooser to pick password, interactive or key.
- **Key login** with an imported key and no stored password went straight to the shell with no overlay.
- **Dropped connections.** A single card at the bottom left counted the dropped sessions, showed a reconnect action, and the sessions came back in their tabs a few seconds later with scrollback kept.
- **Changed host key.** Reconnect stopped at the key check. The screen showed only the new fingerprint, and offered replace, add as new, or cancel.
- **Background output.** A tab that received output while hidden got a small dot.
- **OS detection.** The host card's icon changed to the detected OS after the first connect.
- **Key generation.** Types offered: Ed25519, ECDSA, RSA (including a very short size) and a post-quantum signature type that works only on servers that enable it. Each type had a short compatibility hint. Saving the passphrase was off by default.
- **Key export.** A panel to choose a host, folder and file name, with an advanced section holding the install script.
- **Snippets.** The editor saved as you typed. Running a snippet opened a new tab with a list of targets beside the terminal, and running it again opened a second tab.
- **Workspaces.** Dragging one session tab onto another merged them into a workspace with stacked terminals. Each pane header had a broadcast toggle, maximise and close. With broadcast on, the other panes got a dashed frame.
- **SFTP.** Opening a host needed a double-click in its picker. The actions menu offered open, open with, copy to the other pane, rename, delete, refresh, new folder, hidden files, permissions, select all and close, with selection-only items greyed. Copying showed no progress for a tiny file, and a 200 MB local-to-remote drag finished in seconds on localhost.
- **Menus.** Right-click menus existed on tabs (rename, close), host cards (connect by SSH or SFTP, add to workspace, edit, share, move, copy, duplicate, copy link, remove), key cards and SFTP rows. Terminal panes and pane headers had none.

## 8. Open questions for Mehrdad

1. Is team sharing or sync ever in scope?
2. Side-panel editors everywhere, or keep sheets with drill-in pages (Dalan chose the second in P23)?
3. How far to go on autocomplete?
4. Hardware keys: wanted for the team, or later?
5. Which cloud imports matter?

## 9. Privacy note (for Mehrdad to act on by hand)

During the look, a number of entries from Mehrdad's own known-hosts file were imported into the reference app's cloud-synced vault. They could not be removed by clicking and should be deleted by hand in that app. A few history rows for the test sessions also remain in its logs.
