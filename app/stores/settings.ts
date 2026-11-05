import { defineStore } from 'pinia'
import { toast } from 'vue-sonner'
import { useTauri } from '~/composables/useTauri'

export type Theme = 'system' | 'dark' | 'light'
export type CursorStyle = 'block' | 'bar' | 'underline'

export interface TerminalSettings {
  font_size: number
  cursor: CursorStyle
  cursor_blink: boolean
  scrollback: number
  copy_on_select: boolean
}

const TERMINAL_DEFAULTS: TerminalSettings = { font_size: 14, cursor: 'block', cursor_blink: true, scrollback: 10000, copy_on_select: false }
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

  /** Light or dark right now (resolves "system"). */
  const resolvedTheme = computed(() => (colorMode.value === 'light' ? 'light' : 'dark'))

  function applyTerminal(v: Partial<TerminalSettings>) {
    terminal.font_size = clamp(v.font_size, 8, 32, TERMINAL_DEFAULTS.font_size)
    terminal.scrollback = clamp(v.scrollback, 500, 100000, TERMINAL_DEFAULTS.scrollback)
    terminal.cursor = v.cursor === 'bar' || v.cursor === 'underline' ? v.cursor : 'block'
    terminal.cursor_blink = v.cursor_blink ?? TERMINAL_DEFAULTS.cursor_blink
    terminal.copy_on_select = v.copy_on_select ?? TERMINAL_DEFAULTS.copy_on_select
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

  function setLogs(enabled: boolean) {
    logsEnabled.value = enabled
    save('logs', { enabled })
  }

  return { logsEnabled, setLogs, theme, terminal, autoLockMinutes, loaded, resolvedTheme, load, setTheme, setTerminal, setAutoLock }
})
