import { defineStore } from 'pinia'
import { toast } from 'vue-sonner'
import type { HostSummary, PromptKind, SessionEvent } from '~/types/bindings'
import { CommandError, useTauri } from '~/composables/useTauri'

export type SessionStatus = 'connecting' | 'authenticating' | 'ready' | 'disconnected' | 'error'

export interface Session {
  /** `new` is the S16 New Tab screen: it has no connection until a host is picked. */
  kind: 'session' | 'new' | 'workspace'
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
  /** Key of the workspace this pane belongs to (`''` for a normal tab). */
  parent: string
  /** Typed into the shell once it is ready (run-a-snippet-on-hosts). */
  autorun: string
  // ---- workspace tabs only (S19)
  panes: Session[]
  mode: 'split' | 'focus'
  focus: number
  broadcast: boolean
  /** The id of the saved template this workspace came from, if any. */
  templateId: string
  /** Panes changed since the workspace was opened or saved. */
  dirty: boolean
}

export const MAX_PANES = 16

/** Live SSH tabs (S18). The xterm instance lives in TerminalView; this store only routes bytes to it. */
export const useSessionsStore = defineStore('sessions', () => {
  const api = useTauri()
  const sessions = ref<Session[]>([])
  /** `null` = the Vault view is showing, `'sftp'` = the SFTP tab. */
  const activeKey = ref<string | null>(null)

  // Output that arrives before its terminal is mounted waits here. Not reactive on purpose.
  const writers = new Map<string, (b: Uint8Array) => void>()
  const pending = new Map<string, Uint8Array[]>()
  const lastOpen = new Map<string, number>()

  /** S20: the side panel beside the active terminal (⌘.). */
  const panelOpen = ref(false)
  const active = computed(() => sessions.value.find(s => s.key === activeKey.value) ?? null)
  /** The session keystrokes and snippets go to: the tab itself, or a workspace's focused pane. */
  const activeSession = computed(() => (active.value?.kind === 'workspace' ? (active.value.panes[active.value.focus] ?? null) : active.value))

  /** Types the pending snippet once the shell is ready AND the session id is known (the ready event can beat the open call's answer). */
  function runAutorun(sess: Session) {
    if (!sess.autorun || !sess.id || sess.status !== 'ready') return
    write(sess, new TextEncoder().encode(`${sess.autorun.replace(/\s+$/, '')}\n`))
    sess.autorun = ''
  }

  function onEvent(sess: Session, e: SessionEvent) {
    switch (e.type) {
      case 'state':
        sess.error = null
        if (e.state === 'connecting' || e.state === 'authenticating' || e.state === 'ready') {
          sess.status = e.state
          // The connection may have saved things (last connected, detected OS, a remembered password): reload the cards.
          if (e.state === 'ready' && sess.hostId) setTimeout(() => useHostsStore().refresh(), 1500)
          if (e.state === 'ready') runAutorun(sess)
        }
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
      // No host id = a shell on this computer.
      const { session_id } = sess.hostId
        ? await api.sessionOpen(sess.hostId, 80, 24, e => onEvent(sess, e), b => onData(sess, b))
        : await api.localSessionOpen(80, 24, e => onEvent(sess, e), b => onData(sess, b))
      sess.id = session_id
      runAutorun(sess)
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
    parent: '',
    autorun: '',
    panes: [],
    mode: 'split',
    focus: 0,
    broadcast: false,
    templateId: '',
    dirty: false,
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

  /** A shell on this computer in its own tab. A New Tab that is showing turns into it. */
  function openLocal() {
    const newTab = active.value?.kind === 'new' ? active.value : null
    const sess = newTab ?? blank()
    Object.assign(sess, { kind: 'session', hostId: '', label: 'This computer', address: 'local shell', status: 'connecting' })
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

  /** 1 = the Vault view, 2 = SFTP, 3.. = the open tabs in order (⌘1-⌘9). */
  function activateNth(n: number) {
    if (n === 1) activeKey.value = null
    else if (n === 2) activeKey.value = 'sftp'
    else if (sessions.value[n - 3]) activeKey.value = sessions.value[n - 3]!.key
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

  const workspaceOf = (sess: Session) => sessions.value.find(s => s.key === sess.parent)

  /** Keystrokes from a terminal. In a broadcasting workspace they go to every live pane, not just this one. */
  function input(sess: Session, bytes: Uint8Array) {
    const ws = workspaceOf(sess)
    if (ws?.broadcast) for (const p of ws.panes) write(p, bytes)
    else write(sess, bytes)
  }

  // ---- workspaces (S19): one tab holding several sessions

  function makePane(host: HostSummary, parent: string, autorun = ''): Session {
    const p = blank()
    Object.assign(p, { hostId: host.id, label: host.label, address: `${host.address}:${host.effective_port}`, parent, autorun })
    return p
  }

  /** Opens one tab with a pane per host (up to 16). */
  function openWorkspace(hosts: HostSummary[], opts: { label?: string, mode?: 'split' | 'focus', autorun?: string, templateId?: string } = {}) {
    const list = hosts.slice(0, MAX_PANES)
    if (!list.length) return
    const ws = blank()
    ws.kind = 'workspace'
    ws.status = 'ready'
    ws.label = opts.label ?? `Workspace ×${list.length}`
    ws.mode = opts.mode ?? 'split'
    ws.templateId = opts.templateId ?? ''
    ws.panes = list.map(h => makePane(h, ws.key, opts.autorun ?? ''))
    // A New Tab that is showing is replaced by the workspace.
    const newTab = active.value?.kind === 'new' ? active.value : null
    if (newTab) sessions.value.splice(sessions.value.indexOf(newTab), 1, ws)
    else sessions.value.push(ws)
    activeKey.value = ws.key
    for (const p of ws.panes) connect(p)
    if (hosts.length > MAX_PANES) toast(`A workspace holds ${MAX_PANES} panes. The rest were left out.`)
  }

  function addPane(ws: Session, host: HostSummary) {
    if (ws.panes.length >= MAX_PANES) return void toast(`A workspace holds ${MAX_PANES} panes.`)
    const p = makePane(host, ws.key)
    ws.panes.push(p)
    ws.focus = ws.panes.length - 1
    ws.dirty = true
    connect(p)
  }

  function dropSession(p: Session) {
    if (p.id) api.sessionClose(p.id)
    writers.delete(p.key)
    pending.delete(p.key)
  }

  function closePane(ws: Session, pane: Session): void {
    const i = ws.panes.findIndex(p => p.key === pane.key)
    if (i < 0) return
    dropSession(pane)
    ws.panes.splice(i, 1)
    ws.dirty = true
    if (!ws.panes.length) return close(ws)
    ws.focus = Math.min(ws.focus, ws.panes.length - 1)
  }

  function focusPane(ws: Session, i: number) {
    if (ws.panes[i]) ws.focus = i
  }
  function setMode(ws: Session, mode: 'split' | 'focus') {
    ws.mode = mode
  }
  function toggleMode(ws: Session) {
    setMode(ws, ws.mode === 'split' ? 'focus' : 'split')
  }
  function toggleBroadcast(ws: Session) {
    ws.broadcast = !ws.broadcast
  }

  function close(sess: Session): void {
    const ws = workspaceOf(sess)
    if (ws) return closePane(ws, sess) // a pane that ends closes inside its workspace
    const i = sessions.value.findIndex(s => s.key === sess.key)
    if (i < 0) return
    if (sess.kind === 'workspace') sess.panes.forEach(dropSession)
    else if (sess.kind === 'session') dropSession(sess)
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

  return { sessions, activeKey, active, activeSession, panelOpen, open, openLocal, openNewTab, openWorkspace, addPane, closePane, focusPane, setMode, toggleMode, toggleBroadcast, input, activateNth, attach, detach, write, resize, ack, respond, reconnect, close, clear }
})
