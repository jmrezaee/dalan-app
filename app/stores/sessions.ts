import { defineStore } from 'pinia'
import { toast } from 'vue-sonner'
import type { HostSummary, PromptKind, SessionEvent } from '~/types/bindings'
import { CommandError, useTauri } from '~/composables/useTauri'

export type SessionStatus = 'connecting' | 'authenticating' | 'ready' | 'disconnected' | 'error'

export interface Session {
  /** `new` is the S16 New Tab screen: it has no connection until a host is picked. */
  kind: 'session' | 'new'
  /** Local id for the tab. Stable from the first moment, unlike `id`. */
  key: string
  /** Rust's session id. Empty until `session_open` answers; events can arrive before that. */
  id: string
  hostId: string
  label: string
  address: string
  status: SessionStatus
  prompt: { id: string, kind: PromptKind } | null
  error: { code: string, message: string } | null
  disconnectedAt: Date | null
  /** Jump hosts this connection goes through, e.g. `bastion`. */
  hop: string
}

/** Live SSH tabs (S18). The xterm instance lives in TerminalView; this store only routes bytes to it. */
export const useSessionsStore = defineStore('sessions', () => {
  const api = useTauri()
  const sessions = ref<Session[]>([])
  /** `null` = the Vault view is showing. */
  const activeKey = ref<string | null>(null)

  // Output that arrives before its terminal is mounted waits here. Not reactive on purpose.
  const writers = new Map<string, (b: Uint8Array) => void>()
  const pending = new Map<string, Uint8Array[]>()
  const lastOpen = new Map<string, number>()

  /** S20: the side panel beside the active terminal (⌘.). */
  const panelOpen = ref(false)
  const active = computed(() => sessions.value.find(s => s.key === activeKey.value) ?? null)

  function onEvent(sess: Session, e: SessionEvent) {
    switch (e.type) {
      case 'state':
        sess.error = null
        if (e.state === 'connecting' || e.state === 'authenticating' || e.state === 'ready') sess.status = e.state
        else if (e.state === 'disconnected') {
          sess.status = 'disconnected'
          sess.disconnectedAt = new Date()
        }
        else if (e.state === 'closed') close(sess)
        break
      case 'prompt':
        sess.prompt = { id: e.prompt_id, kind: e.kind }
        break
      case 'error':
        sess.status = 'error'
        sess.error = { code: e.code, message: e.message }
        break
      case 'hop':
        sess.hop = e.via.join(' → ')
        break
      case 'exit':
        break
    }
  }

  function onData(sess: Session, bytes: Uint8Array) {
    const w = writers.get(sess.key)
    if (w) w(bytes)
    else pending.set(sess.key, [...(pending.get(sess.key) ?? []), bytes])
  }

  async function connect(sess: Session) {
    try {
      const { session_id } = await api.sessionOpen(sess.hostId, 80, 24, e => onEvent(sess, e), b => onData(sess, b))
      sess.id = session_id
    }
    catch (e) {
      sess.status = 'error'
      sess.error = { code: e instanceof CommandError ? e.code : 'internal', message: e instanceof Error ? e.message : 'Something went wrong.' }
    }
  }

  const blank = (): Session => reactive<Session>({
    key: crypto.randomUUID(),
    kind: 'session',
    id: '',
    hostId: '',
    label: '',
    address: '',
    status: 'connecting',
    prompt: null,
    error: null,
    disconnectedAt: null,
    hop: '',
  })

  /** Opens a tab for a host and switches to it. A New Tab that is showing turns into the session. */
  function open(host: HostSummary) {
    const now = Date.now()
    if (now - (lastOpen.get(host.id) ?? 0) < 600) return
    lastOpen.set(host.id, now)
    const newTab = active.value?.kind === 'new' ? active.value : null
    const sess = newTab ?? blank()
    Object.assign(sess, { kind: 'session', hostId: host.id, label: host.label, address: `${host.address}:${host.effective_port}`, status: 'connecting' })
    if (!newTab) sessions.value.push(sess)
    activeKey.value = sess.key
    connect(sess)
  }

  /** S16. Reuses a New Tab that is already open instead of piling them up. */
  function openNewTab() {
    const existing = sessions.value.find(s => s.kind === 'new')
    if (existing) {
      activeKey.value = existing.key
      return
    }
    const sess = blank()
    Object.assign(sess, { kind: 'new', label: 'New tab', status: 'ready' })
    sessions.value.push(sess)
    activeKey.value = sess.key
  }

  /** 1 = the Vault view, 2.. = the open tabs in order (⌘1-⌘9). */
  function activateNth(n: number) {
    if (n === 1) activeKey.value = null
    else if (sessions.value[n - 2]) activeKey.value = sessions.value[n - 2]!.key
  }

  /** TerminalView calls this once xterm exists; buffered output is replayed in order. */
  function attach(key: string, write: (b: Uint8Array) => void) {
    writers.set(key, write)
    for (const b of pending.get(key) ?? []) write(b)
    pending.delete(key)
  }
  function detach(key: string) {
    writers.delete(key)
  }

  // Keystrokes and resizes are fire-and-forget; a failure means the session already ended.
  // Keystrokes typed while a prompt or overlay is up must not reach the shell later.
  const write = (s: Session, bytes: Uint8Array) => { if (s.id && s.status === 'ready') api.sessionWrite(s.id, bytes).catch(() => {}) }
  const resize = (s: Session, cols: number, rows: number) => { if (s.id) api.sessionResize(s.id, cols, rows).catch(() => {}) }
  const ack = (s: Session, bytes: number) => { if (s.id) api.sessionAck(s.id, bytes).catch(() => {}) }

  async function respond(sess: Session, accept: boolean, text?: string, remember = false) {
    const p = sess.prompt
    if (!p) return
    sess.prompt = null
    try {
      await api.sessionRespond(sess.id, p.id, { accept, text: text ?? null, remember })
    }
    catch {
      toast.error('That question is no longer open.')
    }
  }

  async function reconnect(sess: Session) {
    sess.status = 'connecting'
    sess.error = null
    try {
      await api.sessionReconnect(sess.id)
    }
    catch (e) {
      sess.status = 'error'
      sess.error = { code: 'internal', message: e instanceof Error ? e.message : 'Could not reconnect.' }
    }
  }

  function close(sess: Session) {
    const i = sessions.value.findIndex(s => s.key === sess.key)
    if (i < 0) return
    if (sess.kind === 'session' && sess.id) api.sessionClose(sess.id)
    writers.delete(sess.key)
    pending.delete(sess.key)
    sessions.value.splice(i, 1)
    if (activeKey.value === sess.key) activeKey.value = sessions.value.at(-1)?.key ?? null
  }

  /** Vault locked: Rust already closed every session, so just forget them. */
  function clear() {
    sessions.value = []
    activeKey.value = null
    writers.clear()
    pending.clear()
  }

  return { sessions, activeKey, active, panelOpen, open, openNewTab, activateNth, attach, detach, write, resize, ack, respond, reconnect, close, clear }
})
