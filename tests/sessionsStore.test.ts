import { beforeEach, describe, expect, it, vi } from 'vitest'
import { computed, nextTick, reactive, ref, watch } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import type { HostSummary, SessionEvent } from '../app/types/bindings'

// The store uses Nuxt auto-imports and talks to Rust through useTauri; both are faked here.
vi.stubGlobal('ref', ref)
vi.stubGlobal('computed', computed)
vi.stubGlobal('reactive', reactive)
vi.stubGlobal('watch', watch)
vi.stubGlobal('useHostsStore', () => ({ refresh: vi.fn() }))
const settingsMock = { terminal: { auto_reconnect: false } }
vi.stubGlobal('useSettingsStore', () => settingsMock)
const reconnects: string[] = []
const reconnectsWith: string[] = []
vi.mock('vue-sonner', () => ({ toast: Object.assign(vi.fn(), { error: vi.fn() }) }))

type Opened = { onEvent: (e: SessionEvent) => void, onData: (b: Uint8Array) => void, id: string }
const opened: Opened[] = []
const written: { id: string, text: string }[] = []
let openDelay = 0

vi.mock('~/composables/useTauri', () => ({
  CommandError: class extends Error {},
  useTauri: () => ({
    sessionOpen: async (_host: string, _c: number, _r: number, onEvent: Opened['onEvent'], onData: Opened['onData']) => {
      const id = `s${opened.length + 1}`
      opened.push({ onEvent, onData, id })
      // Like Tauri: the answer to the open call can arrive after the first events.
      if (openDelay) await new Promise(r => setTimeout(r, openDelay))
      return { session_id: id }
    },
    localSessionOpen: async () => ({ session_id: 'local' }),
    sessionWrite: async (id: string, bytes: Uint8Array) => { written.push({ id, text: new TextDecoder().decode(bytes) }) },
    sessionReconnect: async (id: string) => { reconnects.push(id) },
    sessionReconnectWith: async (id: string, m: string) => { reconnectsWith.push(`${id}:${m}`) },
    sessionClose: vi.fn(),
    sessionResize: vi.fn(),
    sessionAck: vi.fn(),
    sessionRespond: vi.fn(),
  }),
}))

const { useSessionsStore } = await import('../app/stores/sessions')

const host = (n: number): HostSummary => ({ id: `h${n}`, label: `web-${n}`, address: `10.0.0.${n}`, effective_port: 22 }) as unknown as HostSummary
const ready = (o: Opened) => o.onEvent({ type: 'state', state: 'ready' })

beforeEach(() => {
  setActivePinia(createPinia())
  opened.length = 0
  written.length = 0
  reconnects.length = 0
  reconnectsWith.length = 0
  settingsMock.terminal.auto_reconnect = false
  openDelay = 0
})

describe('sessions store', () => {
  it('a prompt puts a question on the tab and a failed connection shows an error', async () => {
    const s = useSessionsStore()
    s.open(host(1))
    await nextTick()
    const tab = s.sessions[0]!
    opened[0]!.onEvent({ type: 'prompt', prompt_id: 'p1', kind: { type: 'password', host: 'web-1', user: 'me' } })
    expect(tab.prompt?.id).toBe('p1')
    opened[0]!.onEvent({ type: 'error', code: 'auth_failed', message: 'nope' })
    expect(tab.status).toBe('error')
    expect(tab.error?.code).toBe('auth_failed')
  })

  it('output in a background tab marks it unread, and opening the tab reads it', async () => {
    const s = useSessionsStore()
    s.open(host(1))
    s.open(host(2))
    await nextTick()
    const [a, b] = s.sessions
    expect(s.activeKey).toBe(b!.key)
    opened[0]!.onData(new Uint8Array([104, 105]))
    expect(a!.unread).toBe(true)
    opened[1]!.onData(new Uint8Array([104]))
    expect(b!.unread).toBe(false) // it is the one on screen
    s.activeKey = a!.key
    await nextTick()
    expect(a!.unread).toBe(false)
  })

  it('a startup script waits for both the ready event and the session id (the event can win the race)', async () => {
    openDelay = 20
    const s = useSessionsStore()
    s.openWorkspace([host(1)], { autorun: 'echo hi' })
    await nextTick()
    ready(opened[0]!) // arrives before sessionOpen has answered
    expect(written).toEqual([])
    await new Promise(r => setTimeout(r, 40))
    expect(written).toEqual([{ id: 's1', text: 'echo hi\n' }])
  })

  it('broadcast sends typing to every live pane and skips the ones that are not connected', async () => {
    const s = useSessionsStore()
    s.openWorkspace([host(1), host(2), host(3)])
    await new Promise(r => setTimeout(r, 5))
    const ws = s.sessions[0]!
    ready(opened[0]!)
    ready(opened[1]!)
    opened[2]!.onEvent({ type: 'error', code: 'host_unreachable', message: 'x' })
    s.toggleBroadcast(ws)
    s.input(ws.panes[0]!, new TextEncoder().encode('ls\n'))
    expect(written.map(w => w.id).sort()).toEqual(['s1', 's2'])
    s.toggleBroadcast(ws)
    written.length = 0
    s.input(ws.panes[0]!, new TextEncoder().encode('pwd\n'))
    expect(written.map(w => w.id)).toEqual(['s1'])
  })

  it('closing the last pane closes the workspace', async () => {
    const s = useSessionsStore()
    s.openWorkspace([host(1), host(2)])
    const ws = s.sessions[0]!
    s.closePane(ws, ws.panes[0]!)
    expect(s.sessions).toHaveLength(1)
    s.closePane(ws, ws.panes[0]!)
    expect(s.sessions).toHaveLength(0)
  })

  it('cycles through Vault, SFTP and the tabs, renames and reorders', async () => {
    const s = useSessionsStore()
    s.open(host(1))
    s.open(host(2))
    const [a, b] = s.sessions
    s.activeKey = null
    s.cycle(1)
    expect(s.activeKey).toBe('sftp')
    s.cycle(1)
    expect(s.activeKey).toBe(a!.key)
    s.cycle(-1) // SFTP
    s.cycle(-1) // Vault
    s.cycle(-1)
    expect(s.activeKey).toBe(b!.key) // wraps from the Vault back to the last tab
    s.rename(a!, '  prod  ')
    expect(a!.label).toBe('prod')
    s.rename(a!, '   ') // blank names are ignored
    expect(a!.label).toBe('prod')
    s.moveTab(b!.key, a!.key, 'before')
    expect(s.sessions.map(x => x.key)).toEqual([b!.key, a!.key])
  })

  it('a workspace holds at most 16 panes', () => {
    const s = useSessionsStore()
    s.openWorkspace(Array.from({ length: 20 }, (_, i) => host(i + 1)))
    expect(s.sessions[0]!.panes).toHaveLength(16)
  })

  it('reconnects by itself after a drop only when the setting is on, with growing waits, and stops after five tries', async () => {
    vi.useFakeTimers()
    try {
      const s = useSessionsStore()
      s.open(host(1))
      await vi.advanceTimersByTimeAsync(5)
      ready(opened[0]!)
      // Off: a drop waits for the user.
      opened[0]!.onEvent({ type: 'state', state: 'disconnected' })
      await vi.advanceTimersByTimeAsync(60_000)
      expect(reconnects).toEqual([])
      // On: first retry after 2 s.
      settingsMock.terminal.auto_reconnect = true
      ready(opened[0]!)
      opened[0]!.onEvent({ type: 'state', state: 'disconnected' })
      await vi.advanceTimersByTimeAsync(1_900)
      expect(reconnects).toEqual([])
      await vi.advanceTimersByTimeAsync(200)
      expect(reconnects).toEqual(['s1'])
      // The server is still down: 4 s, 8 s, 16 s, 30 s, then it gives up.
      for (const wait of [4, 8, 16, 30]) {
        opened[0]!.onEvent({ type: 'error', code: 'host_unreachable', message: 'down' })
        await vi.advanceTimersByTimeAsync(wait * 1000 + 50)
      }
      expect(reconnects).toHaveLength(5)
      opened[0]!.onEvent({ type: 'error', code: 'host_unreachable', message: 'down' })
      await vi.advanceTimersByTimeAsync(120_000)
      expect(reconnects).toHaveLength(5)
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('a wrong password or a changed key never triggers an automatic retry', async () => {
    vi.useFakeTimers()
    try {
      settingsMock.terminal.auto_reconnect = true
      const s = useSessionsStore()
      s.open(host(1))
      await vi.advanceTimersByTimeAsync(5)
      ready(opened[0]!)
      opened[0]!.onEvent({ type: 'state', state: 'disconnected' })
      await vi.advanceTimersByTimeAsync(2_100)
      expect(reconnects).toHaveLength(1)
      opened[0]!.onEvent({ type: 'error', code: 'auth_failed', message: 'nope' })
      await vi.advanceTimersByTimeAsync(120_000)
      expect(reconnects).toHaveLength(1)
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('a server that goes away (exit without a status) is a real drop and reconnects by itself', async () => {
    vi.useFakeTimers()
    try {
      settingsMock.terminal.auto_reconnect = true
      const s = useSessionsStore()
      s.open(host(1))
      await vi.advanceTimersByTimeAsync(5)
      ready(opened[0]!)
      opened[0]!.onEvent({ type: 'exit', status: null } as SessionEvent)
      opened[0]!.onEvent({ type: 'state', state: 'disconnected' })
      expect(s.dropped).toHaveLength(1)
      await vi.advanceTimersByTimeAsync(2_100)
      expect(reconnects).toEqual(['s1'])
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('exit never reconnects by itself and is not counted as a dropped connection', async () => {
    vi.useFakeTimers()
    try {
      settingsMock.terminal.auto_reconnect = true
      const s = useSessionsStore()
      s.open(host(1))
      s.open(host(2))
      await vi.advanceTimersByTimeAsync(5)
      ready(opened[0]!)
      ready(opened[1]!)
      opened[0]!.onEvent({ type: 'exit', status: 0 } as SessionEvent)
      opened[0]!.onEvent({ type: 'state', state: 'disconnected' })
      opened[1]!.onEvent({ type: 'state', state: 'disconnected' })
      expect(s.droppedCard).toHaveLength(0) // only the real drop counts
      await vi.advanceTimersByTimeAsync(60_000)
      expect(reconnects).toEqual(['s2'])
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('two or more dropped sessions raise one card; Reconnect all reconnects each', async () => {
    const s = useSessionsStore()
    s.open(host(1))
    s.open(host(2))
    await nextTick()
    ready(opened[0]!)
    ready(opened[1]!)
    opened[0]!.onEvent({ type: 'state', state: 'disconnected' })
    expect(s.droppedCard).toHaveLength(0) // one drop keeps its own message
    opened[1]!.onEvent({ type: 'state', state: 'disconnected' })
    expect(s.droppedCard).toHaveLength(2)
    s.dismissDropped()
    expect(s.droppedCard).toHaveLength(0)
    ready(opened[0]!)
    await s.reconnectAll()
    expect(reconnects).toEqual(['s2'])
  })

  it('remembers the methods the server accepts and "Try another way" reconnects with just that one', async () => {
    const s = useSessionsStore()
    s.open(host(1))
    await nextTick()
    const tab = s.sessions[0]!
    opened[0]!.onEvent({ type: 'auth_methods', methods: ['password', 'keyboard_interactive'] })
    opened[0]!.onEvent({ type: 'error', code: 'auth_failed', message: 'nope' })
    expect(tab.authMethods).toEqual(['password', 'keyboard_interactive'])
    await s.reconnectWith(tab, 'keyboard_interactive')
    expect(reconnectsWith).toEqual(['s1:keyboard_interactive'])
    expect(tab.status).toBe('connecting')
    opened[0]!.onEvent({ type: 'state', state: 'connecting' }) // a new attempt starts with a clean list
    expect(tab.authMethods).toEqual([])
  })
})
