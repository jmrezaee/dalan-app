// Runs inside the page (bundled by tests/fake/index.ts). Installs a stand-in for Tauri's IPC so the real Nuxt app runs
// in a plain browser, answers every command from the typed table in ./commands, and exposes `window.__fake` for tests.
import { handlers } from './commands'
import { MASTER } from './commands/vault'
import { createModel, type Seed } from './model'
import { scenarios, type ScenarioName } from './scenarios'
import { scripts } from './scripts'
import type { Args, Ctx } from './types'

export interface Options { scenario: ScenarioName, unlocked?: boolean, seed?: Seed }

const camel = (s: string) => s.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase())

export function install(opts: Options) {
  const w = window as unknown as Record<string, any>
  const seed = opts.seed ?? scenarios[opts.scenario]()
  const m = createModel({ ...seed, vault: { exists: true, autoLockMinutes: 15, ...seed.vault, unlocked: opts.unlocked ?? seed.vault?.unlocked ?? false } })

  let counter = 1
  const callbacks = new Map<number, (m: unknown) => void>()
  const channels = new Map<number, number>() // channel id -> next message index
  const listeners = new Map<string, number[]>() // Tauri event name -> callback ids

  // A Channel arrives as an object; its toJSON() gives `__CHANNEL__:<id>`, which is what Rust would see.
  const channel = (v: unknown): number => {
    const s = typeof v === 'string' ? v : (v as { toJSON?: () => string } | null)?.toJSON?.()
    if (typeof s !== 'string' || !s.startsWith('__CHANNEL__:')) throw { code: 'internal', message: 'Expected a channel argument.' }
    return Number(s.slice(12))
  }
  const send = (ch: number, message: unknown) => {
    const index = channels.get(ch) ?? 0
    channels.set(ch, index + 1)
    callbacks.get(ch)?.({ index, message })
  }
  const sendBytes = (ch: number, bytes: Uint8Array) => send(ch, bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength))
  const emitEvent = (name: string, payload: unknown) => {
    for (const id of listeners.get(name) ?? []) callbacks.get(id)?.({ event: name, id, payload })
  }
  const ctx: Ctx = {
    m, channel, send, sendBytes,
    tag: s => m.calls.push(s),
    id: prefix => `${prefix}-${counter++}`,
  }

  const table = handlers as unknown as Record<string, (a: Args, c: Ctx) => unknown>
  const names = Object.keys(table).map(k => k.replace(/[A-Z]/g, c => `_${c.toLowerCase()}`))

  w.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
    transformCallback: (cb: (m: unknown) => void) => { const id = counter++; callbacks.set(id, cb); return id },
    unregisterCallback: (id: number) => { callbacks.delete(id) },
    runCallback: (id: number, msg: unknown) => callbacks.get(id)?.(msg),
    convertFileSrc: (p: string) => p,
    invoke: async (cmd: string, args: Args = {}) => {
      m.calls.push(cmd)
      if (cmd === 'plugin:dialog|open') return '/home/me/ssh_config' // the file picker returns only a path
      if (cmd === 'plugin:event|listen') {
        // `handler` is the callback id Tauri's JS side registered; events are delivered by calling it.
        listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler])
        return counter++
      }
      if (cmd.startsWith('plugin:')) return null
      const failure = m.failNext.get(cmd)
      if (failure) {
        m.failNext.delete(cmd)
        throw failure
      }
      const h = table[camel(cmd)]
      if (!h) {
        m.unhandled.push(cmd)
        throw { code: 'internal', message: `The fake backend has no command "${cmd}".` }
      }
      // A locked vault refuses everything but the few commands that work while locked.
      if (!m.vault.unlocked && !/^(vault_|app_info)/.test(cmd) && !cmd.startsWith('settings_get')) throw { code: 'locked', message: 'Unlock the vault first.' }
      return await h(args, ctx)
    },
  }

  const sessionEvents = (id: string) => {
    const s = m.sessions.get(id)
    if (!s) throw new Error(`No fake session ${id}`)
    return s
  }
  const waitForAnswer = async (sessionId: string, promptId: string) => {
    for (let i = 0; i < 2000; i++) {
      const r = m.responses.find(x => x.id === sessionId && x.prompt === promptId)
      if (r) return r.answer as { accept: boolean, text: string | null, remember: boolean }
      await new Promise(res => setTimeout(res, 10))
    }
    throw new Error(`The UI never answered prompt ${promptId}`)
  }

  const fake = {
    /** What tests read back. These are live views of the model. */
    state: {
      get calls() { return m.calls },
      get responses() { return m.responses },
      get written() { return m.written },
      get unhandled() { return m.unhandled },
      get unlocked() { return m.vault.unlocked },
    },
    master: MASTER,
    commandNames: () => names,
    sessionIds: () => [...m.sessions.keys()],
    /** Server side of a session: a typed event (state, prompt, error, ...). */
    event: (id: string, e: unknown) => send(sessionEvents(id).events, e),
    /** Server side of a session: terminal output as text. */
    output: (id: string, text: string) => {
      const ch = sessionEvents(id).data
      if (ch !== null) sendBytes(ch, new TextEncoder().encode(text))
    },
    /** Plays a scripted server (ubuntu-login, nginx-status, ...). Resolves when the last step has printed. */
    runScript: async (id: string, name: string) => {
      const make = scripts[name]
      if (!make) throw new Error(`No script "${name}"`)
      for (const step of make()) {
        await new Promise(res => setTimeout(res, step.after))
        fake.output(id, step.text)
      }
    },
    /** A prompt the UI must answer; resolves with the answer. */
    prompt: async (id: string, promptId: string, kind: unknown) => {
      fake.event(id, { type: 'prompt', prompt_id: promptId, kind })
      return waitForAnswer(id, promptId)
    },
    /**
     * A believable connect: the host-key question when the key is new or changed, then ready.
     * Returns how it ended: 'ready', 'refused' (the user said no) or 'changed'.
     */
    connect: async (id: string) => {
      const host = m.hosts.find(h => h.id === sessionEvents(id).host)
      if (!host) throw new Error('No host for this session')
      const pattern = host.port ? `[${host.address}]:${host.port}` : host.address
      const presented = m.hostKeys.get(host.id) ?? { key_type: 'ssh-ed25519', fingerprint: `SHA256:fake-${host.label}` }
      const known = m.knownHosts.find(k => k.host_pattern === pattern)
      fake.event(id, { type: 'state', state: 'connecting' })
      if (!known || known.fingerprint !== presented.fingerprint) {
        const kind = known
          ? { type: 'host_key_changed', host: host.label, key_type: presented.key_type, old_fingerprint: known.fingerprint, new_fingerprint: presented.fingerprint }
          : { type: 'host_key_new', host: host.label, key_type: presented.key_type, fingerprint: presented.fingerprint }
        const answer = await fake.prompt(id, ctx.id('p'), kind)
        if (!answer.accept) {
          fake.event(id, { type: 'state', state: 'closed' })
          return known ? 'changed' : 'refused'
        }
        if (answer.remember) {
          m.knownHosts = m.knownHosts.filter(k => k.host_pattern !== pattern)
          m.knownHosts.push({ id: ctx.id('known'), host_pattern: pattern, key_type: presented.key_type, fingerprint: presented.fingerprint, created_at: m.now, last_seen_at: m.now })
        }
      }
      fake.event(id, { type: 'state', state: 'authenticating' })
      fake.event(id, { type: 'state', state: 'ready' })
      host.last_connected_at = Date.now()
      return 'ready'
    },
    /** F11: every live connection drops at once. */
    dropAll: () => {
      for (const [id, s] of m.sessions) if (!s.closed) fake.event(id, { type: 'state', state: 'disconnected' })
    },
    /** F12: the next connect to this host presents a different key. */
    rotateHostKey: (hostId: string, o: { type?: string } = {}) => {
      const cur = m.hostKeys.get(hostId)
      m.hostKeys.set(hostId, { key_type: o.type ? `ecdsa-sha2-nistp256` : (cur?.key_type ?? 'ssh-ed25519'), fingerprint: `SHA256:rotated-${counter++}` })
    },
    failNext: (cmd: string, error: { code: string, message: string }) => { m.failNext.set(cmd, error) },
    /** Tunnels: change a rule's state and tell the UI, like the Rust side does. */
    tunnel: (id: string, state: string, message: string | null = null) => {
      const f = m.forwards.find(x => x.id === id)
      if (!f) throw new Error(`No fake forward ${id}`)
      Object.assign(f, { state, error: message, connections: state === 'active' ? 1 : 0 })
      emitEvent('tunnel-changed', { id, state, connections: f.connections, error: message })
    },
    /** Transfers: drive one by hand after `manualTransfers()`. */
    manualTransfers: (on = true) => { m.manualTransfers = on },
    transferIds: () => [...m.transfers.keys()],
    transfer: (id: string, e: unknown) => send(m.transfers.get(id)!.events, e),
    transferPending: () => [...m.transfers.entries()].map(([id, t]) => ({ id, items: t.items.length, pending: t.pending })),
    /** Edits: pretend the local copy was saved, or that the server file changed. */
    editSaved: () => { for (const e of m.edits.values()) send(e.events, { type: 'changed' }) },
    remoteChanged: (on = true) => { m.editRemoteChanged = on },
    emit: emitEvent,
    lock: () => { m.vault.unlocked = false; emitEvent('vault-locked', { reason: 'idle' }) },
    model: () => JSON.parse(JSON.stringify(m, (_k, v) => (v instanceof Map ? [...v.entries()] : v))),
  }
  w.__fake = fake
}
