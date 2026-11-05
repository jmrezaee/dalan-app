// Keyboard shortcuts: the list of actions, their defaults, and turning a key press into a combo
// string like `Mod+Shift+M`. `Mod` is ⌘ on a Mac and Ctrl elsewhere. Pure functions, easy to test.

export interface ShortcutAction {
  id: string
  label: string
  /** Default combo. */
  combo: string
}

/** Everything that can be rebound. The tab numbers (Mod+1 to Mod+9) stay fixed. */
export const SHORTCUT_ACTIONS: ShortcutAction[] = [
  { id: 'palette', label: 'Command palette', combo: 'Mod+K' },
  { id: 'new-tab', label: 'New tab', combo: 'Mod+T' },
  { id: 'close-tab', label: 'Close tab', combo: 'Mod+W' },
  { id: 'new-host', label: 'New host', combo: 'Mod+N' },
  { id: 'settings', label: 'Settings', combo: 'Mod+,' },
  { id: 'side-panel', label: 'Side panel', combo: 'Mod+.' },
  { id: 'find', label: 'Find in terminal', combo: 'Mod+F' },
  { id: 'broadcast', label: 'Broadcast typing (workspace)', combo: 'Mod+B' },
  { id: 'duplicate-pane', label: 'Duplicate pane (workspace)', combo: 'Mod+D' },
  { id: 'toggle-layout', label: 'Split or focus layout (workspace)', combo: 'Mod+Shift+M' },
  { id: 'save-template', label: 'Save workspace as template', combo: 'Mod+S' },
]

/** Combos the system or every text field already uses. */
const RESERVED = new Set(['C', 'V', 'X', 'A', 'Z', 'Q', 'H', 'R', 'P'].map(k => `Mod+${k}`))

const isMac = () => typeof navigator !== 'undefined' && navigator.platform.toLowerCase().includes('mac')

/** The combo for a key press, or `null` while only modifiers are held. */
export function eventCombo(e: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'>, mac = isMac()): string | null {
  if (['Meta', 'Control', 'Alt', 'Shift', 'AltGraph', 'Dead'].includes(e.key)) return null
  const key = e.key.length === 1 ? e.key.toUpperCase() : e.key
  const mod = mac ? e.metaKey : e.ctrlKey
  const parts: string[] = []
  if (mod) parts.push('Mod')
  // The other primary modifier on this platform is a separate key: Ctrl on a Mac, Meta elsewhere.
  if (mac ? e.ctrlKey : e.metaKey) parts.push('Ctrl')
  if (e.altKey) parts.push('Alt')
  if (e.shiftKey) parts.push('Shift')
  parts.push(key === ' ' ? 'Space' : key)
  return parts.join('+')
}

/** "⌘⇧M" on a Mac, "Ctrl+Shift+M" elsewhere. */
export function formatCombo(combo: string, mac = isMac()): string {
  const parts = combo.split('+')
  // A trailing "+" key splits into an empty last part.
  const key = parts[parts.length - 1] === '' ? '+' : parts[parts.length - 1]!
  const mods = parts.slice(0, parts[parts.length - 1] === '' ? -2 : -1)
  if (!mac) return [...mods.map(m => (m === 'Mod' ? 'Ctrl' : m)), key].join('+')
  const sym: Record<string, string> = { Mod: '⌘', Ctrl: '⌃', Alt: '⌥', Shift: '⇧' }
  return mods.map(m => sym[m] ?? m).join('') + key
}

/** Why a combo can't be used for `id`, or `null` when it is fine. `combos` = action id -> combo in force. */
export function comboProblem(id: string, combo: string, combos: Record<string, string>): string | null {
  if (!combo.startsWith('Mod+')) return 'Use the ' + (isMac() ? '⌘' : 'Ctrl') + ' key in the shortcut.'
  if (/^Mod\+[1-9]$/.test(combo)) return 'Those switch tabs and can\'t be changed.'
  if (RESERVED.has(combo)) return 'That one is used for copy, paste and system actions.'
  const other = SHORTCUT_ACTIONS.find(a => a.id !== id && combos[a.id] === combo)
  return other ? `Already used by "${other.label}".` : null
}

/** The default combo on this platform: plain Ctrl+F belongs to the shell, so find uses Ctrl+Shift+F off a Mac. */
export function defaultCombo(a: ShortcutAction, mac = isMac()): string {
  return a.id === 'find' && !mac ? 'Mod+Shift+F' : a.combo
}

/** Defaults with the user's changes on top. */
export function effectiveCombos(overrides: Record<string, string>, mac = isMac()): Record<string, string> {
  return Object.fromEntries(SHORTCUT_ACTIONS.map(a => [a.id, overrides[a.id] ?? defaultCombo(a, mac)]))
}

/** The action a key press triggers, if any. */
export function actionFor(e: Parameters<typeof eventCombo>[0], combos: Record<string, string>, mac = isMac()): string | null {
  const c = eventCombo(e, mac)
  if (!c) return null
  return SHORTCUT_ACTIONS.find(a => combos[a.id] === c)?.id ?? null
}
