# Design system: Dalan

Inputs are `replica/recon.md` (components and screens S01–S25) and `replica/design/tokens.json`.
Date: 2026-11-04.

## How this was made, and what it is not

- **Nothing was measured from the original app.** `replica/screens/` is empty. This is a system designed for a dense, keyboard-first desktop SSH client: dark by default, with a terminal at the centre. It is built on shadcn-vue conventions. Because no values were measured, nothing here can match the original's trade dress by accident.
- **Brand:** Dalan, with a turquoise accent. See `replica/brand.md` for the name, palette, logo brief and voice. Brand changes only touch token values; the role names stay.
- **Icons:** [Lucide](https://lucide.dev) via `lucide-vue-next` (ISC licence), at 16px in UI and 14px in dense rows, with a 1.75 stroke. OS icons for hosts (Ubuntu, Debian, macOS…) are generic Lucide glyphs plus a letter badge, never distro logos. If you want logos later, check each one's trademark policy.
- **Fonts:** Inter for UI, JetBrains Mono for the terminal and code. Both are SIL OFL. Bundle the woff2 files under `app/assets/fonts/`, since the app runs offline and has no CDN.
- **Copy:** every label and empty state below was written fresh, in the voice in `replica/brand.md` (calm, precise, protective). Where both files have a string, `brand.md` wins.

## Files

| file | what |
| --- | --- |
| `tokens.json` | Source of truth: colour roles for both themes, terminal ANSI palettes, type, space, radius, shadow, motion, layout, z-index. |
| `build_tokens.py` | Writes `tokens.json`. Edit values here or in the JSON. |
| `build_css.py` | Generates `tokens.css` and `terminal-themes.ts` from `tokens.json`. Re-run after every token change. |
| `tokens.css` | CSS variables per theme, shadcn-vue aliases, and the Tailwind v4 `@theme inline` mapping. Tested with Tailwind 4.3. |
| `terminal-themes.ts` | xterm.js `ITheme` objects named `default-dark` and `default-light`. |
| `contrast.py` | WCAG check over the `pairs` list (94 pairs, **0 AA failures**). |
| `preview.html` | Static visual check of the tokens and key components in both themes. Reference only. |

Wiring into Nuxt (M0):

- `app/assets/css/main.css` holds `@import "tailwindcss"; @import "../../../replica/design/tokens.css";` (or a copy kept in sync by a script).
- `@nuxtjs/color-mode` is set to `{ preference: 'dark', fallback: 'dark', classSuffix: '' }`.

## Token rules

- **No raw hex in components.** Use utilities: `bg-bg`, `bg-surface`, `bg-surface-raised`, `bg-surface-hover`, `bg-chrome`, `text-text`, `text-muted`, `border-border`, `border-border-input`, `bg-accent`, `text-on-accent`, `text-danger`, `bg-danger-solid`, `text-success`, `text-warning`.
- **shadcn-vue names work too:** `bg-primary`, `text-primary-foreground`, `bg-popover`, `ring-ring`, `bg-destructive`, `border-input`.
- **One naming clash:** shadcn's own `accent` means "hover background", but ours means the brand colour. When adding a shadcn-vue component, replace `bg-accent`/`text-accent-foreground` (hover) with `bg-surface-hover`/`text-text`. Do it at generation time.
- **Greys:** 5 surfaces (`chrome` < `bg` < `surface` < `surface-raised` < `surface-hover` in dark), 1 border, 1 input border and 2 text levels. Don't add more.
- **Text sizes:** UI text defaults to `text-sm` (13/20). Use `text-xs` (12/16) for meta, `text-2xs` (11/14) for badges and kbd, `text-base` (14/20, 500) for row titles, `text-lg` for sheet and dialog titles, `text-xl` for page titles and `text-2xl` for empty-state and unlock headlines.
- **Spacing** is a 4px base: 2, 4, 6, 8, 12, 16, 20, 24, 32, 40, 48, 64. Control padding is 8/12, and card padding is 12/16. Page gutter `--layout-content-padding` is 24.
- **Radius:**
  - `sm` 4: badges, kbd, checkboxes
  - `md` 6: buttons, inputs, menus
  - `lg` 10: cards, dialogs, palette
  - `xl` 14: unlock card
  - `pill`: tags, status pills
- **Elevation:** in dark mode, separate surfaces with borders, not shadows. Use `shadow-pop` only on floating layers (menus, popovers, dialogs, palette, toasts) and `shadow-drag` on dragged items.
- **Motion:**
  - `fast` 100ms for hover and press
  - `base` 160ms for menus, sheets and tab moves
  - `slow` 240ms for dialog enter
  - Easing is `--motion-ease`. Reduced motion drops transforms.
- **Focus:** every interactive element shows `:focus-visible` as a 2px `accent` outline with a 2px offset. Accent vs `bg` is 6.94:1 (dark) and 5.17:1 (light). Never remove it.
- **Window layout:**
  - Min window is 960×600.
  - The tab bar is 40px high. On macOS it leaves 78px on the left for the traffic lights, with `data-tauri-drag-region` on the empty bar area.
  - The vault nav is 220px and collapses to 56px (icons and tooltips) below 1100px window width.
  - Sheets are 440px and the side panel is 320px.

## Status colours (one meaning each, everywhere)

| meaning | token | used for |
| --- | --- | --- |
| live / unread output / success / tunnel active | `success` | tab dot, rule status, transfer done |
| waiting for you (password, prompt, conflict) | `warning` | tab dot, pending prompt, conflict row |
| failed / disconnected / destructive | `danger` (text), `danger-solid` (filled) | tab dot, error overlay, delete buttons |
| info / selection / focus / primary action | `accent` | buttons, focus ring, selected row bar |

Colour is never the only signal. Every dot has a tooltip and an `aria-label`, and error states carry an icon and text.

---

## Primitives (shadcn-vue, restyled through tokens)

```
Button
  variants  primary (bg accent, text on-accent), secondary (bg surface-hover, text text, border border),
            ghost (transparent, hover surface-hover), danger (bg danger-solid, text on-danger), link (text accent)
  sizes     xs 24px (toolbar icons), sm 28px, md 32px (default), lg 40px (unlock, empty-state CTA); icon-only = square
  states    default, hover (accent-hover / surface-hover), active (translateY 0.5px), focus-visible, disabled (opacity .5, no pointer),
            loading (spinner replaces leading icon, label kept, aria-busy=true, not clickable)
  tokens    radius md, text sm/500, px 12 (md), gap 6, icon 16
  a11y      real <button>; icon-only needs aria-label + Tooltip; Enter/Space activate
  used on   everywhere

SplitButton ("New ▾")
  parts     Button primary + attached chevron Button that opens DropdownMenu
  states    as Button; menu open → chevron rotated 180°
  a11y      chevron has aria-label "More create options", aria-haspopup="menu"
  used on   S02 (Host / Group / Import), S05 (Key / Identity / Certificate), S11 (Snippet / Package)

IconButton
  sizes     24 (dense rows, pane headers), 28 (toolbars)
  states    default (text-muted), hover (bg surface-hover, text text), pressed/toggled (text accent, bg surface-hover), focus, disabled
  a11y      aria-label required; toggles use aria-pressed
  used on   S01, S18, S19, S21

Input / PasswordInput / NumberInput / Textarea
  size      32px high, text sm, px 10, radius md
  colours   bg bg (dark) / bg (light), border border-input, text text, placeholder text-muted
  states    default, hover (border text-muted), focus (border accent + ring 2px accent/35%), invalid (border danger + message below in danger, aria-invalid),
            disabled (bg surface, opacity .6), readonly, inherited (ghost value in text-muted italic "inherited from Production"; typing overrides)
  extras    PasswordInput: eye toggle (aria-pressed), "saved" state shows ●●●●●● and a "Change" link, because the secret never comes back from Rust
            NumberInput (ports): 1–65535, monospace digits, arrow-key step
  a11y      every input has a visible <Label>; errors linked via aria-describedby
  used on   S03, S04, S06, S07, S08, S10, S12, S23, S24, S25

Select / Combobox
  as Input; Combobox (Reka) for long lists with type-ahead: groups, identities, keys, snippets, hosts
  states    + empty list ("No identities yet — create one") with an inline create action
  used on   S03, S04, S07, S10

Switch / Checkbox / RadioGroup
  switch 28×16, checkbox 16 radius sm, accent when on; label clickable; focus ring on the control
  used on   S03 (agent forwarding), S06 (save passphrase), S10 (type), S16 (multi-select), S23

Tabs (in-page)
  underline style: text-muted → text on active, 2px accent bar; 32px high
  a11y      Reka Tabs: arrow keys move, roving tabindex
  used on   S20, S23

Tooltip
  bg surface-raised, text xs, border border, shadow pop, 500ms delay, 0 delay when moving between tooltips
  shows keyboard shortcut as Kbd on the right
  used on   every icon-only control, tab dots, collapsed vault nav

Kbd
  text 2xs mono, px 4, h 18, radius sm, bg surface-hover, border border; shows ⌘ on macOS, Ctrl elsewhere
  used on   S17, tooltips, S23 shortcuts

Badge / TagChip
  Badge: text 2xs/500, h 18, px 6, radius sm, bg surface-hover, text text-muted
  TagChip: radius pill, h 20, text xs; removable variant has an × IconButton (aria-label "Remove tag dev")
  used on   S02, S03, S05 (key type), S14

Dialog / AlertDialog
  width     sm 400, md 520, lg 640; radius lg; bg surface-raised; shadow pop; overlay rgba(0,0,0,.5)
  parts     title (text lg), description (text-muted), body, footer right-aligned (secondary, primary)
  states    default, submitting (primary loading, Esc disabled), error banner inside
  AlertDialog for destructive: focus starts on Cancel; the danger button names the action ("Delete 3 hosts")
  a11y      Reka Dialog: focus trap, Esc closes, focus returns to trigger, aria-labelledby/describedby
  used on   S06, S08, S13, S22 conflicts, all deletes

Sheet (side editor)
  right, 440px, full height under the tab bar, bg surface-raised, border-l border, shadow pop
  parts     header (title + close), scrollable body with sections, sticky footer (Cancel / Save)
  states    clean, dirty (Save enabled; closing asks "Discard changes?"), saving, validation errors (scroll to first)
  keyboard  ⌘S / Ctrl+S saves when focus is inside the sheet; Esc closes (asks if dirty)
  used on   S03, S04, S07, S12

DropdownMenu / ContextMenu
  bg surface-raised, border border, shadow pop, radius md, item h 28, text sm, px 8, icon 16 text-muted
  item states  default, highlighted (bg surface-hover), disabled, destructive (text danger), checked, submenu
  shortcut column right-aligned (Kbd)
  a11y      Reka menus: arrows, typeahead, Esc; context menus also open with Shift+F10 / the Menu key on the focused item
  used on   S02, S05, S11, S14, S21, S01 tabs

Toast
  bottom-right, 360 wide, bg surface-raised, border border, shadow pop, left 3px bar in status colour
  variants  success, error (stays until dismissed if it has an action), info
  a11y      role=status (info/success), role=alert (error); never the only place an error is shown
  used on   global (saved, copied, tunnel started, transfer done)

EmptyState
  centred in content, max 360 wide: Lucide icon 32 in text-muted, title text-lg/600, one line text-muted, primary Button, optional secondary link
  copy (ours)
    S02  "No servers yet" · "Add one by hand or bring them over from your SSH config." · [Add host] [Import]
    S05  "Your keychain is empty" · "Generate a key or import one you already use." · [Generate key]
    S09  "No tunnels" · "Forward a remote port to your machine, or the other way round." · [New rule]
    S11  "No snippets" · "Save commands you type often and run them on many servers." · [New snippet]
    S14  "No trusted servers yet" · "Fingerprints appear here after your first connection."
    S15  "No logs yet" · "Each server keeps its latest session here. Bookmark one to keep it."
    S16  "Nothing recent" · "Connections you open will show up here."
  used on   S02, S05, S09, S11, S14, S15, S16

Skeleton
  bg surface-hover, radius md, pulse 1.2s (static under reduced motion)
  used on   first load of S02 and S21 (only if > 150ms; otherwise render directly)
```

## App components

```
TabBar (S01)
  layout    h 40, bg chrome, border-b border; mac inset 78 left; [Vault][SFTP] fixed, then session tabs (scroll overflow with fade + chevrons), [+], drag region fills the rest
  Tab       min 120 / max 220, h 32, radius md md 0 0, text sm, px 10; leading icon or StatusDot, label (truncate), close × on hover/active
            states  default (text-muted), hover (bg surface-hover), active (bg bg, text text, joins the content), focus-visible, dragging (shadow drag, opacity .9),
                    drop-target-for-workspace (2px dashed accent outline + "Drop to split" hint), unsaved workspace (filled dot replaces ×, ⌘S saves)
  keyboard  ⌘T new, ⌘W close, ⌘1–9 jump, ⌃Tab / ⌃⇧Tab cycle, ⌘J tab switcher, middle-click closes, F2 rename
  a11y      role=tablist, tabs role=tab with aria-selected; the close button is a separate focusable button with aria-label "Close web-1"

StatusDot
  8px circle; success (unread output), warning (needs input, gentle 1.5s pulse), danger (error/disconnected), none (idle)
  aria-label "web-1: waiting for input" etc.; never colour-only (tooltip + label)
  used on   S01, S09, S16, S19

VaultNav (inside the Vault tab)
  220px column, bg surface, items h 32: Hosts, Keychain, Port forwarding, Snippets, Known hosts, Logs (icon + label + count badge)
  states    default, hover, active (bg surface-hover, text text, 2px accent bar left), collapsed (56px icons + tooltips)
  a11y      <nav aria-label="Vault"> with links; arrow keys not required

Toolbar (S02, S05, S09, S11, S14, S15, S21)
  h 48, px 24, border-b border; left: page title (text xl) or breadcrumb; right: SearchInput, view toggle, SplitButton

SearchInput
  Input with Lucide search icon and clear ×; ⌘F focuses the page search; Esc clears then blurs
  shows result count for screen readers (aria-live polite) when filtering

GroupCard / HostCard (S02, S16, S17)
  grid      auto-fill min 240, gap 12; list view = rows h 44
  HostCard  bg surface, border border, radius lg, p 12; [OS badge 32] label (text base) / user@address:port (text xs text-muted, mono for address) / tag chips; trailing kebab on hover
            states  default, hover (border text-muted/40, bg surface-raised), selected (2px accent ring), multi-selected (checkbox visible), connecting (spinner in OS badge),
                    connected (small success dot on badge), drag source (opacity .5), focus-visible
  GroupCard folder icon + label + "12 hosts" meta; drop-target state (bg accent/10, border accent)
  interaction  click selects, double-click / Enter connects, Space toggles multi-select with ⌘/Ctrl/Shift, right-click → ContextMenu
  a11y      list is a grid (role=grid) with roving focus; each card announces "web-1, deploy at 10.0.0.5, group Production, tags dev"

CredentialPicker (S03, S04, S07)
  a Combobox that lists: "Password", identities, keys (with type badge) + "Generate key…" / "Import key…" footer actions
  shows inherited value as ghost text with "from Production" Badge; "Override" link clears inheritance; "Use inherited" link restores
  never shows secret values

TagInput (S03)
  chips inside an input; Enter or comma adds; Backspace on empty removes last; suggestions popover from existing tags (Combobox)

ChainPicker (S03, S04)
  ordered list of hosts with drag handles + "Add jump host" Combobox; shows "you → bastion → web-1" preview line in text-xs mono
  validation: cannot include itself; warns on cycles

EnvVarList (S03, S04)
  rows NAME (mono, validated [A-Za-z_][A-Za-z0-9_]*) = value, lock toggle to store a value as a secret, remove ×; mode select: inherit / merge / replace
  hint under list: "The server must allow these names with AcceptEnv."

HostPicker / HostMultiPicker (S08, S10, S13, S21)
  Combobox; multi variant = checklist with search, group headers, "Select all in group", selected count

KeyCard / IdentityCard (S05)
  like HostCard; KeyCard shows type Badge (ED25519/RSA-4096), fingerprint (SHA256:… truncated middle, mono xs), "passphrase saved" icon, cert Badge
  actions   copy public key (toast "Public key copied"), export to host, edit, delete

RuleCard (S09)
  row h 56: type icon (L/R/D), label, "localhost:5432 → db.internal:5432 via bastion" (mono xs), status pill, toggle button
  states    stopped (text-muted), starting (spinner), active (success pill + open connection count), reconnecting (warning), error (danger + message: "Port 5432 is already in use on this computer. Pick another, or stop whatever is using it.")
  double-click toggles; Enter toggles when focused

Stepper (S10)
  3 steps (Type → Where → Review) with "Skip wizard" link to a single form; step dots + labels; Back/Next in footer

CommandDialog (S17)
  640 wide, top 15vh, bg surface-raised, radius lg, shadow pop; Input h 44 text base; groups: Hosts, Open tabs, Snippets, Actions
  item h 36: icon, label with matched characters in text weight 600, meta right (text-muted xs), Kbd for actions
  states    empty query (recent hosts + common actions), results, no results ("No matches. Press ⌘N to add a host"), loading not needed (client-side)
  a11y      Reka Combobox in dialog: arrow keys, Enter opens, ⌘Enter opens in split, Esc closes, aria-activedescendant

NewTab (S16)
  centred column 560: SearchInput (autofocus; typing filters), "Recent" list with checkboxes on hover, "Open workspace (3)" primary when ≥2 checked, shortcuts row below

TerminalView (S18, S19, S15 replay)
  xterm.js filling its container; padding 8 (in theme bg); font mono 14/1.2; cursor block blinking (setting); webgl renderer with canvas fallback
  theme from terminal-themes.ts, per host/group override
  read-only variant for log replay (S15): no cursor, selection only

StatusOverlay (S18)
  centred card over a dimmed terminal (bg bg/80 + blur 2px): icon, title, one line, actions. Our copy:
    connecting        spinner · "Connecting to web-1…" · hop line "via bastion" · [Cancel]
    host-key new      "First time connecting to web-1" · "Check that this fingerprint matches the server:" SHA256 in mono with copy button · [Cancel] [Trust and connect] (primary)
    host-key changed  danger · "This server's identity changed" · "That can happen after a reinstall, or it can mean someone is intercepting the connection. Don't continue unless you expected it." old/new fingerprints · [Close] (focused) · "Replace saved key…" (link → AlertDialog that asks you to type the host name)
    password / passphrase  PasswordInput + "Remember" checkbox · [Cancel] [Continue]
    keyboard-interactive   the server's prompts as fields
    auth failed       "Sign-in failed" · reason · [Edit host] [Try again]
    unreachable       "Can't reach 10.0.0.5:22" · "Check the address and port, your VPN, or a firewall in between." · [Retry]
    disconnected      "Connection closed" · "web-1 disconnected at 14:32." · [Reconnect] (Enter) [Close tab]
  a11y      overlay is role=alertdialog when it needs an answer; focus moves into it; terminal gets aria-hidden while it shows

Workspace / PaneHeader (S19)
  split tree via Reka/shadcn Resizable; handle 4px hit area (1px border visible, accent on hover/drag); min pane 200×120; up to 16 panes
  PaneHeader h 28, bg surface, text xs: StatusDot, title, broadcast badge, maximize, close
  states    focus pane (accent 1px inset border), broadcasting (all panes get a warning-coloured 2px top bar + "Broadcasting to 4" pill in the tab), focus mode (one large + others listed in the side panel)
  keyboard  ⌘D split right, ⌘⇧D split down, ⌘⌥arrows move focus, ⌘B broadcast, ⌘⇧M split/focus, ⌘S save template

SidePanel (S20)
  320 wide right panel inside the session area, bg surface, border-l; Tabs: Snippets / History / Theme; ⌘. toggles
  snippet rows: label + first line of script (mono xs, truncated); Enter pastes and runs, ⌥Enter pastes without running

FileTable + PathBar (S21)
  two panes split 50/50 (resizable); each: pane header (endpoint picker: "This computer" or host Combobox), PathBar, FileTable, footer (n items, selection size)
  PathBar   breadcrumb segments (click to navigate, last one refreshes); click empty area → editable Input with full path; Enter goes; invalid path → danger text under it
  FileTable rows h 28: icon (folder/file/symlink), name, size (right-aligned, tabular nums), modified (local time), permissions (mono, rwxr-xr-x); sortable headers (aria-sort)
            states  loading (skeleton rows), empty folder ("This folder is empty"), permission denied (lock icon + "You don't have access to this folder"), SFTP disabled ("SFTP isn't enabled on this server"),
                    selected (bg accent/15), multi-select, drag source, drop target (pane border accent dashed + "Copy to /var/www"), renaming (inline Input)
  keyboard  arrows, Enter opens folder / file, Backspace or ⌘↑ goes up, F2 rename, Delete → AlertDialog, ⌘A, ⌘C/⌘V copy across panes, ⌘⇧. hidden files
  virtualised list over 500 rows

TransferDrawer (S22)
  bottom drawer in the SFTP tab, collapsed to a 32px bar "3 transfers · 42%" (click to expand, max 40% height)
  ProgressRow: direction icon, name, from → to (xs muted), progress bar (h 4, accent; success when done; danger on fail), speed, ETA, cancel ×
  ConflictDialog: "notes.txt already exists in /home/deploy" · sizes and dates side by side · [Skip] [Keep both] [Replace] + "Do this for all" checkbox

DatabasesView (S26)                                                              [Redis, replica/redis/spec.md]
  PageToolbar "Databases" + muted "Redis"; filter Input (220 wide, "Filter databases"); primary "New database"
  table: Name (3px environment bar: warning = production, border-input = staging, none; name medium; environment word xs),
         Address (mono xs, muted), Route ("Direct" or "bastion → db-primary"), Kind, Writes (pill: Read-only muted, Confirm writes accent, Open warning)
  rows h 44; hover bg surface; Enter or double-click opens the database in a tab; row menu: Open, Edit, Delete
  note under the table (xs muted): "A database can be reached directly, or through any saved host. Passwords are kept in the vault, like your SSH passwords."
  states    loading (skeleton rows), empty (EmptyState, Database icon: "No databases yet. Add one to browse its keys, run commands and watch channels." + New database),
            no filter match ("No database matches cache."), delete → AlertDialog "Delete cache-prod?" (Cancel focused, danger-solid "Delete database")
  keyboard  Tab reaches the filter, the button and each row; ↑/↓ move between rows; Enter opens; the row menu opens with the context-menu key

DatabaseSheet (S27)
  Sheet 440 wide, title "New database" / "Edit database", footer h 56: [Test connection] result text … [Cancel] [Save] (primary)
  fields    Name · Environment (Segmented None / Staging / Production + hint "Production shows an amber bar on the tab, so nobody mistakes it for staging.")
            Kind (Segmented Single server / Cluster / Sentinel; Cluster: "Give the address of any node. Dalan asks it where the others are and reaches each one the same way.";
                  Sentinel: the address field becomes "Sentinels" (a comma list), the port "Default port" (26379), plus "Primary's name" with "The name the sentinels know the primary by.") ·
            Reach it (Segmented Directly / Through a host) · Host (select of saved SSH hosts; hint "Dalan signs in to db-primary and talks to Redis from there. Nothing to forward by hand.")
            Address ("Address", or "Address, as seen from the host") + Port (96 wide, mono) · Kind (Segmented Single server / Cluster / Sentinel; the last two disabled until they are built)
            Username (optional) + Password (PasswordInput; "Saved in the vault" + Change when one is saved; "Remove saved password")
            Use TLS (Switch, "certificate is checked") + "Certificate name (optional)" when on
            Writes (radio cards): Read-only "Look, never change. Write commands are refused before they are sent." · Confirm writes "Every change shows what it will do and asks first." (default)
                                  · Open "Edits go straight through. Dangerous commands still ask."
  states    saving (Save shows a spinner), validation error (role=alert under the form, the server's sentence), testing (Test connection spinner),
            test ok (success text "Connected · Redis 7.2 · 3 ms", plus "· replica" when it is one), test failed (danger text, one line), no hosts yet ("No hosts yet. Add one under Hosts first.")
  keyboard  Enter in a field saves; Esc closes; the radio cards are one radiogroup (arrows move)

RedisTab (S28 shell; the views arrive in R2 to R5)
  toolbar h 44, border-b: Database icon, name (medium), environment chip ("production" warning / "staging" muted), route text (xs muted: "Direct" or "bastion → db-primary"),
           logical database picker ("db0 · 12,408 keys"; hidden for Cluster), the four views (Browser, Console, Pub/Sub, Server; aria-current on the open one),
           latency dot + "3 ms" (success under 100 ms, warning under 500, danger above; "…" until the first ping),
           write-mode button (lock icon + the mode's name; opens WriteModeDialog; disabled on a replica)
  banner   Read-only mode: lock icon + "Read-only. Nothing here can change data." + link "Change…" (opens WriteModeDialog)
  a production database adds a 2px warning line on top of its tab
  states    connecting (ConnectSteps, centred), a host question (PromptPanel in the same card: host key, password, passphrase, 2FA),
            failed (one line of what happened + one useful action: auth → Edit database; unreachable → Try again; certificate → Edit database; host problems → Try again),
            dropped ("The connection dropped" + "Trying again in 4 s (2 of 5)." + Reconnect now), replica banner ("This server is a replica. It only accepts reads."),
            ready (the open view; until R2 an EmptyState "Connected to cache-prod" with the server's version)
  keyboard  the views are a tablist (←/→ move, Enter opens); the tab stays mounted while another tab shows

RedisTypeChip
  mono 10px semibold uppercase, 1px border in its own colour, min-width 44, centred: str (ansi-blue), hash (ansi-magenta), list (ansi-green),
  set (ansi-yellow), zset (ansi-red), stream (ansi-cyan), json (ansi-white); any other type shows the server's word in muted.
  The chip always carries the text, so colour is never the only signal.

KeyTree (S28, left side, 320 wide, resizable later)
  toolbar: pattern Input (mono, placeholder "user:*", Enter scans; empty = every key), type Select ("All types", then the seven), Refresh (IconButton),
           New key (IconButton, Plus; hidden in Read-only; opens NewKeyDialog)
  tree     role="tree", rows h 28, virtualised; built from the scanned keys by the database's separator
           folder row: chevron, "user:" (mono), count right (muted); a folder with a single key under it is shown as that key
           key row: RedisTypeChip, name (mono, the part after the folder; binary keys escaped as \xNN), time left right ("23h", "12m", "no expiry" muted)
           selected key: bg accent/10; focus ring on the focused row
  footer   "Scanned 2,000 of 12,408 keys" + [Scan more] + [Bulk…] (ghost; hidden in Read-only; opens BulkDialog)
           was: "Scanned 2,000 of 12,408 keys" + [Scan more] (hidden when the whole keyspace was walked: "All 12,408 keys scanned")
  states   scanning (footer spinner "Scanning…"), empty database ("This database has no keys."), no match ("No keys match user:* in what's been scanned so far." + Scan more),
           error (danger line with the server's sentence + Try again)
  keyboard ↑/↓ move, → opens a folder (or moves into it), ← closes it (or moves to its parent), Enter opens a key or toggles a folder, Home/End; one Tab stop

KeyHeader (S28, right side top)
  RedisTypeChip, key name (mono semibold, truncates, full name in title), Copy name (IconButton, toast "Key name copied"),
  expiry text ("Expires in 23h 14m" / "No expiry"), size ("8 fields · 412 B", "48,210 members", "39 bytes"), Reload (IconButton)
  Expiry control: the expiry text is a button; it opens an inline Input ("90s, 15m, 2h, 7d or a date") + [Set expiry] + [Remove expiry] (only when it has one) + [Cancel];
                  a value it can't read shows "Use 90s, 15m, 2h, 7d or a date in the future." in danger; Enter sets, Esc cancels
  Rename: the name becomes an Input + [Rename] + [Cancel]; never overwrites ("A key with that name already exists."); binary key names can't be renamed here
  Delete key: ghost button in danger text. Confirm mode: WriteConfirmDialog; over 10,000 items: DangerConfirmDialog; Open mode: an AlertDialog first ("Delete maintenance?")
  states   Read-only: the expiry is plain text and Rename / Delete key are hidden

Value viewers (S28 hash, S29 the rest): the right side under the header. Each loads 500 items at a time.
  shared   footer "Showing 500 of 48,210" + [Load more]; loading (skeleton rows); empty ("This hash has no fields." etc.); key gone ("This key is gone. It expired or was deleted." + Close);
           error (danger line + Try again); binary cells are shown as hex in muted with a "binary" tag; long cells truncate with the full value in a title
  ValueHash    filter Input ("Filter fields", filters what is loaded) · table Field | Value (mono), virtualised
  ValueString  Segmented Text / JSON / Hex (JSON only when the text parses; binary opens as Hex and the other two are disabled) · mono pre, wraps ·
               footer "39 bytes · text" · over 1 MB: "Showing the first 64 KB of 2.0 MB." + [Load all]
  ValueJson    for JSON-module keys and for strings that parse: Segmented Tree / Text; Tree is the value pretty-printed with keys, strings and numbers coloured (ansi-blue, ansi-green, ansi-yellow)
  ValueList    table # | Value, positions from 0
  ValueSet     table Member
  ValueZset    table Score ↓ | Member, highest first
  ValueStream  table ID | Fields ("order=5512 status=paid"), newest first · footer "Groups: billing (0 waiting), emails (3 waiting)." when there are any
  keyboard     the table is one Tab stop; ↑/↓ move the focused row; Enter edits it, Delete removes it, Esc cancels an edit, Enter in the edit saves
  editing      (R3; hidden in Read-only) one item at a time. The focused or hovered row shows Edit (Pencil) and Remove (Trash) IconButtons at its right end.
               Editing turns the row into Inputs for the cells that can change + [Cancel] + primary [Review change] (Confirm mode) or [Save] (Open mode).
               Adding shows the same row at the top of the table. Binary cells can be removed, not edited.
    ValueHash    [Add field]; edit changes the value; remove → "Remove this field?"
    ValueList    [Start] [End] add at either end; edit changes the value; remove is by position, and the item is checked first ("That item changed since you looked.")
                 hint "Add to the start or the end. Remove by position."
    ValueSet     [Add member]; remove; no edit · hint "Members are unique; adding one twice changes nothing."
    ValueZset    [Add member] (score + member); edit changes the score; remove
    ValueStream  [Add entry] (ID, empty = the server chooses; fields as "order=5512 status=paid"); remove; no edit
    ValueString  Text view is a textarea; footer [Discard] + [Review change] / [Save], enabled once the text differs; previews (over 1 MB) and binary values are read-only
    ValueJson    JSON-module keys: the Text view is the editor; invalid text shows "Line 6: this isn't valid JSON yet." in danger and disables the button

WriteConfirmDialog (S33 "Confirm a write"; Confirm mode, and heavy commands in every mode)
  AlertDialog, 440 wide. Title asks what will happen ("Change this field?", "Remove this member?", "Delete this key?").
  grid (label xs muted, 64 wide | value): Database ("cache-prod" + warning "· production") · Target (mono: "user:1042:profile › plan") ·
       Now (mono danger; left out when there was nothing) · After (mono success; left out for removals) · Items ("48 fields go with it", deletes only)
  the exact command in a mono block (bg, rounded 6, wraps; long arguments are shortened by Rust)
  footer [Cancel] (focused) [primary: the action's verb, "Change" / "Add field" / "Remove" / "Delete key" (danger-solid for removals and deletes)]
  states   committing (button loading), refused (the dialog closes and a toast says why: expired after two minutes, already used, the mode changed)
  keyboard Esc and Cancel close without sending anything; Enter on the focused Cancel cancels

DangerConfirmDialog (S33 "Dangerous command"; every mode that isn't Read-only)
  AlertDialog with a danger border. Title with a warning icon ("Delete leaderboard:weekly on cache-prod?"), one sentence of what is lost
  ("This removes the key and its 48,210 members. It can't be undone, and everyone using this database is affected."), the exact command in a mono block,
  label "Type cache-prod to confirm" + Input (mono, no autocomplete)
  footer [Cancel] (focused) [danger-solid: the action, disabled until the text is exactly the database's name]. Rust compares the name again.

WriteModeDialog (S33; the toolbar's mode button and the banner's "Change…")
  Dialog "Writes on cache-prod": WriteModePicker (the three modes as radios: title + one line each; the same picker as the editor)
  making a production database less protected adds PasswordInput "Master password" + hint "Making a production database less protected needs your master password."
  footer [Cancel] [Change mode] (disabled while the mode is unchanged)
  states   wrong password ("That password didn't work." under the field), too many tries (the cooldown sentence), saving
  In the editor (S27) a saved production database's looser modes are disabled with the hint "Loosen this from the mode button in its tab (it asks for your master password)."

NewKeyDialog (S28; KeyTree's New key)
  Dialog "New key": Name (mono) · Type (Select: String, Hash, List, Set, Sorted set, Stream, JSON) · the first item for that type
  (String: Value textarea · Hash and Stream: Field + Value · List: First item · Set: First member · Sorted set: Member + Score · JSON: a textarea, checked) ·
  Expiry (optional; same words as the expiry control)
  footer [Cancel] [Create key] (or [Review change] in Confirm mode). Never overwrites: "A key with that name already exists."

RedisConsole (S30)
  our own input and output (not xterm), on the terminal background: mono 12/20, output selectable
  output   one block per command: prompt "cache-prod:0>" (accent) + the line as typed, then the reply as Rust printed it:
           "(integer) 83640", quoted strings, "1) 2) 3)" lists with nested lists indented; list numbers and "(nil)" muted, "(integer)" in ansi-yellow,
           "(error) ..." in danger; a muted "· 0.4 ms" after the reply. Over 1,000 lines: the first 1,000 + [Show all 4,210 lines]
  states   running (muted "…"), not sent (danger: "Not sent. cache-prod is read-only in Dalan."),
           a write in Confirm mode (accent/8 block: "This changes data on production." [Send] [Cancel]),
           a heavy command in every mode (warning/10 block: "Not sent. KEYS walks every key at once and can stall a busy server."
           + link "Use SCAN instead" (puts the safe form in the input) + "Run anyway…"),
           a dangerous command (DangerConfirmDialog), queued inside MULTI ("QUEUED"), cancelled (muted "Cancelled. Nothing was sent.")
  input    prompt + Input (mono, no border) + hint "↑ history · ⌘L clear" (Ctrl+L off the Mac)
           completion popup above the input while the first word is typed: up to 6 commands, name (mono semibold) + syntax (mono muted),
           the first highlighted (accent/15), footer = its summary + "Tab to complete"; after the name, one line with that command's syntax
  History  aside 300 wide: "History" + "this database" + [Clear]; rows h 26 mono, newest first, click puts the line in the input;
           footer "History is kept encrypted, like terminal history. Commands that carry a password (AUTH, ACL SETUSER, CONFIG SET requirepass) are never saved."
           empty: "Nothing yet. Commands you run show up here."
  keyboard Enter runs, ↑/↓ walk the history, Tab completes, Esc closes the popup, ⌘L / Ctrl+L clears the output; the output is role="log" aria-live="polite"

RedisServer (S31)
  header   "Server" (14 semibold) + "updated 2 s ago" (xs muted) + [Refresh every 5 s] (secondary, aria-pressed) + Refresh now (IconButton)
           refreshes only while this view shows, the tab is in front and the window has the focus
  tiles    six in a row (surface, border, radius 10; label caps xs muted, value 20/28 semibold tabular, note xs muted):
           Version ("7.2.4" · "standalone") · Uptime ("41 days" · "since Oct 5") · Memory ("1.8 GB" · "of 4.0 GB · 45%" or "no limit set") ·
           Clients ("212" · "0 blocked") · Commands ("8,420 /s" · "now") · Hit rate ("96.2%" · "of 1,000 lookups" or "—" · "no lookups yet")
  panels   Slow commands (When | Took | Command | Client; newest first; [Clear log…] ghost, hidden in Read-only → DangerConfirmDialog;
           footer "Read from the server's own slow log. Dalan never turns on live command tracing."; empty "Nothing slow has been logged.")
           Keys by database (Database | Keys | With expiry | Avg. time left; "—" when none expire; empty "This server holds no keys.")
           Nodes (Cluster and Sentinel only, under Keys by database): Role (primary / replica / sentinel, muted) | Address (mono) |
           Slots ("0–5460", or "of 10.0.2.11:7000" for a replica; the column is "Follows" for Sentinel) | State (ok success, catching up warning, anything else danger)
           note beside the title: "the tiles above are one node's; keys are the whole cluster's" / "as the sentinels see them"
  states   loading (skeleton tiles), error (danger line + Try again), a limited user (tiles show what INFO gave; the slow log is empty)

RedisPubSub (S32)
  left     "Listening to" aside, 300 wide: Input (mono, "channel or pat*ern") + primary [Listen]; a name with *, ? or [ is a pattern
           rows h 36: dot (success while listening, border-input when stopped), name (mono), "channel" / "pattern" (xs muted), count right (tabular), × "Stop listening to orders.*"
           footer "Listening uses its own connection, so the Browser and Console keep working. It stops when you close the tab or lock Dalan."
           empty "Not listening to anything yet."; error under the input (danger, one line: an old server, 50 names already)
  right    header h 40: "Messages" + "131 received · keeping the last 5,000" (+ "· 240 dropped" when a flood or a long pause lost some) +
           filter Input (200, "Filter messages", filters what is kept) + [Pause] (secondary, aria-pressed; "Resume" while paused) + [Clear]
           table Time (mono muted, 14:02:11.482) | Channel (mono ansi-cyan) | Message (mono; binary as \xNN), newest first, virtualised, rows h 30
           Pause keeps receiving into a buffer of 5,000 and says "Paused · 312 waiting"; Resume shows them
           empty "Nothing has arrived yet." (listening) / "Listen to a channel to see its messages here." (not listening)
  publish  bar on surface, hidden in Read-only: "PUBLISH" caps + channel Input (180, mono) + message Input (mono) +
           [Review and publish] (Confirm mode → WriteConfirmDialog "Publish this message?") or [Publish] (Open mode); toast "Published."
  states   the connection dropped (dots go grey; listening starts again by itself once the tab is back)
  keyboard Enter in the name field listens; Enter in the message field publishes; the table is one Tab stop

BulkDialog (S33 "Bulk action"; KeyTree's [Bulk…], hidden in Read-only)
  Dialog "Actions on matching keys", 520 wide
  row      pattern Input (mono, search icon, "session:*") + type Select ("All types" …) + [Preview]
  action   Segmented Delete / Set expiry / Remove expiry; Set expiry adds an Input ("90s, 15m, 2h, 7d or a date")
  preview  scanning ("Scanning… 2,400 found" + [Stop]) → panel: "3,940 keys match" + "whole database scanned" (muted) +
           the first 20 names (mono muted, " · " between) + "… 3,920 more"; none: "No keys match session:*."; stopped: "Stopped before the whole database was scanned. Preview again to run."
           changing the pattern or the type clears the preview
  confirm  label "Type cache-prod to confirm" + Input (mono); the action button (danger-solid: "Delete 3,940 keys", "Set expiry on 3,940 keys",
           "Remove expiry from 3,940 keys") is disabled until there is a whole preview, the name matches exactly and the expiry reads
  running  progress bar (accent on surface-hover, h 6) + "Deleting 1,655 of 3,940 · in small batches, so the server stays responsive" + [Stop]
  done     "Deleted 3,940 keys." / "Stopped. 1,500 of 3,940 done; the rest were left alone." + [Close]; the key tree is scanned again
  footer note "Stop keeps what's already done." Cancel has the focus when the dialog opens. Rust refuses a bulk action in Read-only and checks the name itself.

ConnectSteps (S33 "Connecting")
  card: title "Opening cache-prod"; one row per step h 28: state icon (done = success check, active = spinning accent ring, pending = empty ring, failed = danger ×),
        label ("Sign in to bastion", "Reach Redis at 127.0.0.1:6379", "Sign in as app-ro"), detail (xs muted: "through bastion", the SSH client's progress line, or why it failed)
  footer: [Cancel] (closes the tab); the list is aria-live="polite" so each finished step is announced

ShortcutRecorder (S23)
  Input-like button "Press keys…"; shows captured combo as Kbd chips; conflict → danger text "Already used by New tab" + [Replace]

UnlockCard (S24)
  centred on bg, 380 wide, radius xl, bg surface, p 32: app mark (from /replica-brand), title text-2xl, PasswordInput lg, primary lg button, caps-lock warning (warning text)
  first-run variant: password + confirm + strength meter (5 bars: danger → success) + "There is no way to recover this password. Write it down." (warning)
  wrong password: shake 160ms (none under reduced motion) + danger text; after 5 tries, 30s cooldown with countdown
```

## Copy voice (ours)

- Short, plain and calm. Talk about servers and connections rather than nodes and instances.
- Buttons say what they do: "Trust and connect", "Delete 3 hosts", "Copy to /var/www". Never use a bare "OK".
- Errors say what happened, then what to try: "Can't reach 10.0.0.5:22. Check the address and port, or your VPN."
- Use sentence case everywhere. Spell out shortcuts only in tooltips and the palette.

## Build the primitives first (M0, before any screen)

Add a dev-only route `app/pages/design.vue` (guarded by `import.meta.dev`). It renders every primitive and app component above in every state, in both themes side by side (`.dark` wrapper plus a light wrapper), with a terminal sample using both palettes.

Check it on every PR that touches UI:

- Screenshot it with Playwright at 1280×800 and compare against the last run.
- Run `python3 replica/design/contrast.py replica/design/tokens.json` in CI. It exits 1 on any AA failure.
