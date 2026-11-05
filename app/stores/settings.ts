import { defineStore } from 'pinia'
import { toast } from 'vue-sonner'
import { useTauri } from '~/composables/useTauri'
import { SHORTCUT_ACTIONS, comboProblem, defaultCombo, effectiveCombos } from '~/utils/shortcuts'

export type Theme = 'system' | 'dark' | 'light'
export type CursorStyle = 'block' | 'bar' | 'underline'

export interface TerminalSettings {
  font_size: number
  cursor: CursorStyle
  cursor_blink: boolean
  scrollback: number
  copy_on_select: boolean
  /** Empty = the bundled JetBrains Mono. Any installed monospace family name. */
  font_family: string
  /** Theme for servers that have none of their own; empty = the app's light or dark default. */
  default_theme: string
  /** Reconnect by itself after a drop (2, 4, 8, 16, 30 s, then stop). Off until switched on. */
  auto_reconnect: boolean
}

const TERMINAL_DEFAULTS: TerminalSettings = { font_size: 14, cursor: 'block', cursor_blink: true, scrollback: 10000, copy_on_select: false, font_family: '', default_theme: '', auto_reconnect: false }
const clamp = (n: unknown, lo: number, hi: number, fallback: number) => (typeof n === 'number' && Number.isFinite(n) ? Math.min(hi, Math.max(lo, Math.round(n))) : fallback)

/** S23. Loaded from Rust after unlock, saved back on change. Terminals read `terminal` live. */
export const useSettingsStore = defineStore('settings', () => {
  const api = useTauri()
  const colorMode = useColorMode()
  const theme = ref<Theme>('dark')
  const terminal = reactive<TerminalSettings>({ ...TERMINAL_DEFAULTS })
  const autoLockMinutes = ref(15)
  const loaded = ref(false)
  /** Keep a log of each session (S15). Off until chosen. */
  const logsEnabled = ref(false)
  /** Ask a server which OS it runs, once, for its icon. On unless switched off. */
  const detectOs = ref(true)
  /** ML-KEM hybrid key exchange in the SSH handshake. On unless switched off. */
  const postQuantum = ref(true)
  /** Path of the shell for "This computer"; empty = your own ($SHELL). */
  const localShell = ref('')
  /** Shortcuts the user changed: action id -> combo (see utils/shortcuts). */
  const shortcutOverrides = ref<Record<string, string>>({})
  const combos = computed(() => effectiveCombos(shortcutOverrides.value))
  /** Remember typed commands for the side panel's History tab. Off until chosen. */
  const historyEnabled = ref(false)

  /** Light or dark right now (resolves "system"). */
  const resolvedTheme = computed(() => (colorMode.value === 'light' ? 'light' : 'dark'))

  function applyTerminal(v: Partial<TerminalSettings>) {
    terminal.font_size = clamp(v.font_size, 8, 32, TERMINAL_DEFAULTS.font_size)
    terminal.scrollback = clamp(v.scrollback, 500, 100000, TERMINAL_DEFAULTS.scrollback)
    terminal.cursor = v.cursor === 'bar' || v.cursor === 'underline' ? v.cursor : 'block'
    terminal.cursor_blink = v.cursor_blink ?? TERMINAL_DEFAULTS.cursor_blink
    terminal.copy_on_select = v.copy_on_select ?? TERMINAL_DEFAULTS.copy_on_select
    terminal.font_family = typeof v.font_family === 'string' ? v.font_family.slice(0, 80) : ''
    terminal.default_theme = typeof v.default_theme === 'string' ? v.default_theme : ''
    terminal.auto_reconnect = v.auto_reconnect === true
  }

  async function load() {
    try {
      const all = await api.settingsGetAll()
      const parse = (k: string) => { try { return JSON.parse(all[k] ?? '{}') as Record<string, unknown> } catch { return {} } }
      const a = parse('appearance')
      theme.value = a.theme === 'light' || a.theme === 'system' ? a.theme : 'dark'
      colorMode.preference = theme.value
      applyTerminal(parse('terminal') as Partial<TerminalSettings>)
      const sec = parse('security')
      if (typeof sec.auto_lock_minutes === 'number') autoLockMinutes.value = sec.auto_lock_minutes
      logsEnabled.value = parse('logs').enabled === true
      const hostsDoc = parse('hosts')
      detectOs.value = hostsDoc.detect_os !== false
      postQuantum.value = hostsDoc.post_quantum !== false
      localShell.value = typeof hostsDoc.local_shell === 'string' ? hostsDoc.local_shell : ''
      const ov = parse('shortcuts').overrides
      shortcutOverrides.value = ov && typeof ov === 'object' ? Object.fromEntries(Object.entries(ov).filter(([id, c]) => SHORTCUT_ACTIONS.some(a => a.id === id) && typeof c === 'string')) as Record<string, string> : {}
      historyEnabled.value = parse('history').enabled === true
      loaded.value = true
    }
    catch {
      // Plain browser or locked: keep the defaults.
    }
  }

  async function save(key: string, value: object) {
    try {
      await api.settingsSet(key, JSON.stringify(value))
    }
    catch {
      toast.error('Couldn\'t save that setting. Try again.')
    }
  }

  function setTheme(t: Theme) {
    theme.value = t
    colorMode.preference = t
    save('appearance', { theme: t })
  }
  function setTerminal(patch: Partial<TerminalSettings>) {
    applyTerminal({ ...terminal, ...patch })
    save('terminal', { ...terminal })
  }
  function setAutoLock(minutes: number) {
    autoLockMinutes.value = minutes
    useVaultStore().autoLockMinutes = minutes
    save('security', { auto_lock_minutes: minutes })
  }

  /** `null` goes back to the default. Returns a reason when the combo can't be used. */
  function setShortcut(id: string, combo: string | null): string | null {
    // Copy without this action's entry; add it back below if it differs from the default.
    const next = Object.fromEntries(Object.entries(shortcutOverrides.value).filter(([k]) => k !== id))
    if (combo !== null && combo !== defaultCombo(SHORTCUT_ACTIONS.find(a => a.id === id)!)) {
      const problem = comboProblem(id, combo, effectiveCombos(next))
      if (problem) return problem
      next[id] = combo
    }
    shortcutOverrides.value = next
    save('shortcuts', { overrides: next })
    return null
  }

  // The `hosts` document holds three switches; every change saves all of them.
  const saveHosts = () => save('hosts', { detect_os: detectOs.value, post_quantum: postQuantum.value, local_shell: localShell.value })
  function setDetectOs(on: boolean) {
    detectOs.value = on
    saveHosts()
  }
  function setPostQuantum(on: boolean) {
    postQuantum.value = on
    saveHosts()
  }
  function setLocalShell(path: string) {
    localShell.value = path.trim()
    saveHosts()
  }

  function setLogs(enabled: boolean) {
    logsEnabled.value = enabled
    save('logs', { enabled })
  }

  function setHistory(enabled: boolean) {
    historyEnabled.value = enabled
    save('history', { enabled })
  }

  return { detectOs, setDetectOs, postQuantum, setPostQuantum, localShell, setLocalShell, shortcutOverrides, combos, setShortcut, logsEnabled, setLogs, historyEnabled, setHistory, theme, terminal, autoLockMinutes, loaded, resolvedTheme, load, setTheme, setTerminal, setAutoLock }
})
