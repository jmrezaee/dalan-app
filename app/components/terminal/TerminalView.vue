<script setup lang="ts">
// S18. One xterm per session. It stays mounted while the session lives (hard rule 4):
// hiding a tab is `v-show` in the parent, never unmounting.
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebglAddon } from '@xterm/addon-webgl'
import '@xterm/xterm/css/xterm.css'
import { terminalThemes } from '~/utils/terminal-themes'
import type { Session } from '~/stores/sessions'

const props = defineProps<{ session: Session, active: boolean }>()
const sessions = useSessionsStore()
const settings = useSettingsStore()
const el = ref<HTMLElement | null>(null)

let term: Terminal | undefined
let fit: FitAddon | undefined
let observer: ResizeObserver | undefined
const encoder = new TextEncoder()

const hosts = useHostsStore()
/** The host's (or its group's) chosen theme, else the default for the app's light/dark mode. */
const themeFor = (t: 'dark' | 'light') => terminalThemes[hosts.themeFor(props.session.hostId) ?? ''] ?? terminalThemes[t === 'light' ? 'default-light' : 'default-dark']

function refit() {
  // A hidden tab has no size; fitting then would collapse the terminal to 0 columns.
  if (!el.value || el.value.clientWidth === 0) return
  fit?.fit()
}

onMounted(() => {
  term = new Terminal({
    fontFamily: '"JetBrains Mono Variable", "JetBrains Mono", ui-monospace, monospace',
    fontSize: settings.terminal.font_size,
    lineHeight: 1.2,
    cursorBlink: settings.terminal.cursor_blink,
    cursorStyle: settings.terminal.cursor,
    scrollback: settings.terminal.scrollback,
    allowProposedApi: true,
    theme: themeFor(settings.resolvedTheme),
  })
  // ⌘ shortcuts (new tab, palette, ...) belong to the app: tell xterm to ignore them so they reach the window.
  term.attachCustomKeyEventHandler(e => !e.metaKey)
  term.onSelectionChange(() => {
    const sel = term?.getSelection()
    if (settings.terminal.copy_on_select && sel) navigator.clipboard.writeText(sel).catch(() => {})
  })
  fit = new FitAddon()
  term.loadAddon(fit)
  term.open(el.value!)
  try {
    const gl = new WebglAddon()
    gl.onContextLoss(() => gl.dispose()) // falls back to the DOM renderer
    term.loadAddon(gl)
  }
  catch { /* no WebGL: the default renderer is fine */ }

  term.onData(d => sessions.input(props.session, encoder.encode(d)))
  term.onBinary(d => sessions.input(props.session, Uint8Array.from(d, c => c.charCodeAt(0))))
  term.onResize(({ cols, rows }) => sessions.resize(props.session, cols, rows))

  // Report how many bytes xterm has actually drawn, so Rust can pause the server when we fall behind.
  sessions.attach(props.session.key, b => term!.write(b, () => sessions.ack(props.session, b.length)))

  observer = new ResizeObserver(refit)
  observer.observe(el.value!)
  refit()
  if (props.active) term.focus()
})

onBeforeUnmount(() => {
  observer?.disconnect()
  sessions.detach(props.session.key)
  term?.dispose()
})

// Settings and theme changes apply to a live terminal.
watch(() => settings.terminal, (t) => {
  if (!term) return
  term.options.fontSize = t.font_size
  term.options.cursorStyle = t.cursor
  term.options.cursorBlink = t.cursor_blink
  term.options.scrollback = t.scrollback
  nextTick(refit)
}, { deep: true })
watch([() => settings.resolvedTheme, () => hosts.themeFor(props.session.hostId)], () => { if (term) term.options.theme = themeFor(settings.resolvedTheme) })

watch(() => props.active, (a) => {
  if (!a) return
  nextTick(() => {
    refit()
    term?.focus()
  })
})
// Once the session id is known, tell Rust the real size (it opened at 80x24).
watch(() => props.session.id, (id) => {
  if (id && term) sessions.resize(props.session, term.cols, term.rows)
})
// Hand the keyboard back to the terminal after an overlay closes.
watch(() => props.session.status, (s) => {
  if (s === 'ready') {
    if (term) sessions.resize(props.session, term.cols, term.rows)
    if (props.active) term?.focus()
  }
})
</script>

<template>
  <div ref="el" class="size-full bg-terminal-bg p-2" />
</template>
