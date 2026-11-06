// A fake Rust backend for browser tests. `installFakeBackend(page)` puts a stand-in for Tauri's IPC in the page,
// so the real Nuxt app runs in a plain browser: commands answer from a small in-memory model, and the test
// drives the "server side" of a session through `window.__fake` (events and terminal bytes).
// Only what the tests need is faked; any other command is recorded in `__fake.unhandled` and answers `null`.
import type { Page } from '@playwright/test'

export interface FakeHost { id: string, label: string, address: string, port?: number }

/** Runs inside the page. Keep it plain JavaScript: it is stringified and sent to the browser. */
function backend(initial: { hosts: FakeHost[], unlocked: boolean }) {
  const w = window as unknown as Record<string, any>
  let nextId = 1
  const callbacks = new Map<number, (m: unknown) => void>()
  const channels = new Map<number, number>() // channel id -> next message index
  const state = {
    unlocked: initial.unlocked,
    hosts: initial.hosts,
    sessions: new Map<string, { events: number, data: number, host: string }>(),
    written: [] as { id: string, text: string }[],
    responses: [] as { id: string, prompt: string, answer: any }[],
    unhandled: [] as string[],
    calls: [] as string[],
  }
  // A Channel arrives as an object; its toJSON() gives `__CHANNEL__:<id>`, which is what Rust would see.
  const channelId = (v: any) => {
    const s = typeof v === 'string' ? v : v?.toJSON?.()
    return typeof s === 'string' && s.startsWith('__CHANNEL__:') ? Number(s.slice(12)) : null
  }
  const push = (ch: number, message: unknown) => {
    const index = channels.get(ch) ?? 0
    channels.set(ch, index + 1)
    callbacks.get(ch)?.({ index, message })
  }
  const summary = (h: FakeHost) => ({
    id: h.id, label: h.label, address: h.address, port: h.port ?? null, username: 'me', has_password: true, group_id: null, tags: [],
    effective_username: 'me', effective_port: h.port ?? 22, effective_has_password: true, key_id: null, identity_id: null,
    effective_key_id: null, startup_snippet_id: null, jump_mode: 'inherit', jump_host_ids: [], effective_jump_host_ids: [],
    inherited: { username: null, port: null, password: null, key: null, jump: null, startup_snippet: null },
    last_connected_at: null, os_hint: null, protocol: 'ssh',
  })
  const handlers: Record<string, (a: any) => unknown> = {
    vault_status: () => ({ exists: true, unlocked: state.unlocked, auto_lock_minutes: 15, cooldown_secs: null }),
    vault_unlock: (a) => {
      if (a.masterPassword !== 'correct-horse') throw { code: 'wrong_password', message: 'That password didn\'t work.' }
      state.unlocked = true
      return null
    },
    vault_lock: () => { state.unlocked = false; return null },
    vault_touch: () => null,
    import_preview: () => ({
      rows: [
        { label: 'api', address: 'api.example.com', port: 2200, username: 'deploy', group: null, tags: [], jump: ['bastion'], duplicate: false, warnings: [] },
        { label: 'web-1', address: '10.0.0.1', port: null, username: 'me', group: null, tags: [], jump: [], duplicate: true, warnings: [] },
        { label: 'odd', address: '10.9.9.9', port: null, username: null, group: null, tags: [], jump: [], duplicate: false, warnings: ['ProxyCommand was ignored.'] },
      ],
      warnings: ['An Include line was not followed.'],
    }),
    import_apply: (a) => { state.calls.push(`import:${a.rows.length}`); return { created: a.rows.length, skipped: 0 } },
    fs_home: () => '/home/me',
    snippets_list: () => ({ snippets: [], packages: [] }),
    hosts_tree: () => ({ groups: [], hosts: state.hosts.map(summary) }),
    terminal_themes_list: () => ({ hosts: [], groups: [] }),
    tags_list: () => [],
    recent_connections: () => [],
    settings_get_all: () => ({}),
    session_open: (a) => {
      const id = `fake-${nextId++}`
      state.sessions.set(id, { events: channelId(a.events)!, data: channelId(a.data)!, host: a.hostId })
      return { session_id: id }
    },
    session_write: (a) => { state.written.push({ id: a.sessionId, text: new TextDecoder().decode(new Uint8Array(a.data ?? [])) }); return null },
    session_respond: (a) => { state.responses.push({ id: a.sessionId, prompt: a.promptId, answer: a.answer }); return null },
    session_reconnect: (a) => { state.calls.push(`reconnect:${a.sessionId}`); return null },
    session_reconnect_with: (a) => { state.calls.push(`reconnect:${a.sessionId}:${a.method}`); return null },
    session_close: (a) => { state.sessions.delete(a.sessionId); return null },
    session_resize: () => null,
    session_ack: (a) => { state.calls.push(`ack:${a.bytes}`); return null },
  }
  const LISTY = /_(list|all|get_all)$|^(keys|identities|snippets|packages|forwards|logs|known_hosts|templates)/
  w.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
    transformCallback: (cb: (m: unknown) => void) => { const id = nextId++; callbacks.set(id, cb); return id },
    unregisterCallback: (id: number) => { callbacks.delete(id) },
    runCallback: (id: number, m: unknown) => callbacks.get(id)?.(m),
    convertFileSrc: (p: string) => p,
    invoke: async (cmd: string, args: any = {}) => {
      state.calls.push(cmd)
      if (cmd === 'plugin:dialog|open') return '/home/me/ssh_config' // the file picker returns only a path
      if (cmd.startsWith('plugin:')) return cmd.includes('listen') ? nextId++ : null
      const h = handlers[cmd]
      if (h) return h(args)
      state.unhandled.push(cmd)
      return LISTY.test(cmd) ? [] : null
    },
  }
  w.__fake = {
    state,
    /** Server side of a session: a typed event (state, prompt, error, ...). */
    event: (sessionId: string, e: unknown) => push(state.sessions.get(sessionId)!.events, e),
    /** Server side of a session: terminal output. */
    output: (sessionId: string, text: string) => {
      const ch = state.sessions.get(sessionId)!.data
      const bytes = new TextEncoder().encode(text)
      const index = channels.get(ch) ?? 0
      channels.set(ch, index + 1)
      callbacks.get(ch)?.({ index, message: bytes.buffer })
    },
    sessionIds: () => [...state.sessions.keys()],
  }
}

export async function installFakeBackend(page: Page, hosts: FakeHost[], unlocked = false) {
  await page.addInitScript(`(${backend.toString()})(${JSON.stringify({ hosts, unlocked })})`)
}
