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
