import { defineStore } from 'pinia'
import { toast } from 'vue-sonner'
import type { RedisBytes, RedisDbKeys, RedisKeyInfo_Serialize, RedisKeyRow, RedisPending_Serialize, RedisWriteOp_Deserialize, RedisCommandInfo_Serialize, RedisConsoleReply_Serialize, RedisServerStats, RedisSlowEntry, RedisBulkAction, RedisBulkEvent, RedisMessage_Serialize, RedisSubEvent_Serialize, RedisPlanStep_Serialize, PromptKind, RedisEvent_Serialize, RedisServerInfo, RedisStepState, RedisValuePage_Serialize } from '~/types/bindings'
import { bytesId, bytesText, textBytes } from '~/utils/redisBytes'
import { foldersOf } from '~/utils/redisTree'
import { useTauri } from '~/composables/useTauri'

export type RedisStatus = 'connecting' | 'ready' | 'dropped' | 'failed' | 'closed'
/** `note` is the step's fixed remark ("through bastion"); `detail` is what shows now: a progress line, why it failed, or the note. */
/** How many keys one `SCAN` call asks the server to look at (replica/redis/spec.md). */
export const SCAN_COUNT = 500

/** The value of the open key, as far as it has been loaded. One shape per type; pages are appended. */
export type LoadedValue =
  | { type: 'string', value: RedisBytes, totalBytes: number, truncated: boolean }
  | { type: 'json', text: string }
  | { type: 'hash', fields: { field: RedisBytes, value: RedisBytes }[] }
  | { type: 'list', items: RedisBytes[] }
  | { type: 'set', members: RedisBytes[] }
  | { type: 'zset', members: { member: RedisBytes, score: string }[] }
  | { type: 'stream', entries: { id: string, fields: { field: RedisBytes, value: RedisBytes }[] }[] }
  | { type: 'unsupported', keyType: string }

/** What a confirmation asks, in the editor's words: the title, the button's verb, and whether it destroys something. */
export interface WriteAsk { title: string, action: string, text?: string, danger?: boolean }

/** How many Pub/Sub messages a tab keeps (and how many wait while it is paused). */
export const KEEP_MESSAGES = 5000
export interface RedisSub { name: string, pattern: boolean, count: number, live: boolean }

/** One command in the console's output, with what came of it. */
export interface ConsoleEntry {
  id: number
  prompt: string
  line: string
  state: 'running' | 'done' | 'refused' | 'pending' | 'cancelled'
  lines: string[]
  error: boolean
  truncated: boolean
  elapsedMs: number | null
  showAll: boolean
  /** Held by the gate until "Send" or "Run anyway": a write in Confirm mode, or a heavy read in any mode. */
  ask: { pendingId: string, heavy: boolean, safer: string | null } | null
}

export interface RedisStep { label: string, note: string | null, detail: string | null, state: 'pending' | RedisStepState }

function setup() {
  const api = useTauri()
  const dbId = ref('')
  /** Rust's connection id. Empty until `redis_open` answers; events can arrive before that. */
  const connId = ref('')
  const status = ref<RedisStatus>('closed')
  const steps = ref<RedisStep[]>([])
  /** A question from the SSH sign-in of a hop (host key, password, passphrase, 2FA). */
  const prompt = ref<{ id: string, kind: PromptKind } | null>(null)
  const info = ref<RedisServerInfo | null>(null)
  const error = ref<{ code: string, message: string } | null>(null)
  const dropped = ref<{ attempt: number, of: number, retryInSecs: number } | null>(null)
  const latencyMs = ref<number | null>(null)
  const selected = ref(0)
  /** Keys per logical database, as far as the server told us. */
  const keys = ref<Record<number, number>>({})
  /** Which of the tab's views is open. They switch on this ref, not on a route. */
  const view = ref<'browser' | 'console' | 'pubsub' | 'server'>('browser')
  /** The saved database's key separator (`:` unless changed), for the tree. */
  const db = ref<{ separator: string } | null>(null)
  const setSeparator = (separator: string) => { db.value = { separator } }
  // ---- Browser (F14): the keys scanned so far, and the open key with its value in pages. Nothing here writes.
  const pattern = ref('')
  const typeFilter = ref<string | null>(null)
  const keyRows = ref<RedisKeyRow[]>([])
  /** `''` before the first page, `'0'` once the whole keyspace has been walked. */
  const scanCursor = ref('')
  /** How many keys the server has looked at for this pattern (the sum of the COUNT hints), for "Scanned 2,000 of 12,408". */
  const scanned = ref(0)
  const scanning = ref(false)
  const scanError = ref('')
  const scanDone = computed(() => scanCursor.value === '0')
  const openFolders = ref<Set<string>>(new Set())
  const selectedKey = ref<RedisBytes | null>(null)
  const selectedId = computed(() => (selectedKey.value ? bytesId(selectedKey.value) : ''))
  const keyInfo = ref<RedisKeyInfo_Serialize | null>(null)
  const value = ref<LoadedValue | null>(null)
  /** The cursor of the next page of the open value; `null` when everything is loaded. */
  const valueNext = ref<string | null>(null)
  const valueLoading = ref(false)
  const valueError = ref<{ code: string, message: string } | null>(null)
  // Each scan and each key load has a number: an answer that comes back after a newer request is dropped.
  let scanRun = 0
  let valueRun = 0

  // An attempt number: events from a connection that was replaced (Try again) are ignored.
  let generation = 0

  function onEvent(gen: number, e: RedisEvent_Serialize) {
    if (gen !== generation) return
    switch (e.type) {
      case 'plan':
        steps.value = e.steps.map((s: RedisPlanStep_Serialize) => ({ label: s.label, note: s.detail, detail: s.detail, state: 'pending' }))
        if (status.value !== 'dropped') status.value = 'connecting'
        break
      case 'step': {
        const s = steps.value[e.index]
        if (s) Object.assign(s, { state: e.state, detail: e.detail ?? (e.state === 'active' ? s.detail : s.note) })
        break
      }
      case 'host':
        if (e.event.type === 'prompt') prompt.value = { id: e.event.prompt_id, kind: e.event.kind }
        break
      case 'ready':
        info.value = e.info
        selected.value = e.info.selected
        keys.value = Object.fromEntries(e.info.databases.map((d: RedisDbKeys) => [d.index, d.keys ?? 0]))
        latencyMs.value = e.info.latency_ms
        status.value = 'ready'
        void scan()
        void relisten()
        error.value = null
        dropped.value = null
        prompt.value = null
        break
      case 'latency':
        latencyMs.value = e.ms
        break
      case 'dropped':
        status.value = 'dropped'
        subs.value = subs.value.map(x => ({ ...x, live: false }))
        dropped.value = { attempt: e.attempt, of: e.of, retryInSecs: e.retry_in_secs }
        latencyMs.value = null
        prompt.value = null
        break
      case 'failed':
        status.value = 'failed'
        error.value = { code: e.code, message: e.message }
        dropped.value = null
        prompt.value = null
        break
      case 'closed':
        if (status.value !== 'failed') status.value = 'closed'
        break
    }
  }

  /** Opens (or re-opens) the tab's connection. Progress arrives as events. */
  async function open(id: string) {
    const gen = ++generation
    if (connId.value) void api.redisClose(connId.value)
    dbId.value = id
    connId.value = ''
    status.value = 'connecting'
    steps.value = []
    prompt.value = null
    error.value = null
    dropped.value = null
    info.value = null
    latencyMs.value = null
    try {
      const opened = await api.redisOpen(id, e => onEvent(gen, e))
      if (gen === generation) {
        connId.value = opened.conn_id
        // "Ready" can arrive before this answer does; the first scan needs the id, so start it here in that case.
        // (The cast: TypeScript still thinks the status is the 'connecting' set above; an event changed it meanwhile.)
        if ((status.value as RedisStatus) === 'ready' && scanCursor.value === '' && !scanning.value) void scan()
      }
      else void api.redisClose(opened.conn_id) // closed or replaced while opening
    }
    catch (e) {
      if (gen !== generation) return
      status.value = 'failed'
      error.value = { code: (e as { code?: string }).code ?? 'internal', message: e instanceof Error ? e.message : 'Couldn\'t open the database.' }
    }
  }

  /** "Try again" and "Reconnect now": a fresh attempt from the start. */
  const retry = () => (dbId.value ? open(dbId.value) : Promise.resolve())

  /** The answer to the host's question. `accept: false` is Cancel, which fails the attempt. */
  async function respond(accept: boolean, text: string | null = null, remember = false) {
    const p = prompt.value
    if (!p || !connId.value) return
    prompt.value = null
    await api.redisRespond(connId.value, p.id, { accept, text, remember }).catch(() => {})
  }

  /** The logical database picker. */
  async function select(index: number) {
    if (!connId.value || status.value !== 'ready') return
    try {
      const k = await api.redisSelect(connId.value, index)
      selected.value = k.index
      keys.value = { ...keys.value, [k.index]: k.keys ?? 0 }
      void scan() // another logical database: start the tree again
    }
    catch (e) {
      error.value = { code: (e as { code?: string }).code ?? 'internal', message: e instanceof Error ? e.message : 'Couldn\'t switch the database.' }
    }
  }

  const message = (e: unknown, fallback: string) => (e instanceof Error ? e.message : fallback)

  /** Starts the key list again from the top (a new pattern, a new type filter, Refresh, another database). */
  async function scan() {
    scanRun++
    keyRows.value = []
    scanCursor.value = ''
    scanned.value = 0
    scanError.value = ''
    closeKey()
    await scanMore()
  }

  /** One more `SCAN` page. A pattern that matches little can come back empty: the footer still moves on. */
  async function scanMore() {
    if (!connId.value || status.value !== 'ready' || scanDone.value || scanning.value) return
    const run = scanRun
    scanning.value = true
    scanError.value = ''
    try {
      const page = await api.redisScan(connId.value, scanCursor.value, pattern.value.trim(), typeFilter.value, SCAN_COUNT)
      if (run !== scanRun) return
      // SCAN may return a key twice while the keyspace changes: keep the first.
      const seen = new Set(keyRows.value.map(r => bytesId(r.key)))
      keyRows.value = [...keyRows.value, ...page.keys.filter(r => !seen.has(bytesId(r.key)))]
      scanCursor.value = page.cursor
      scanned.value += SCAN_COUNT
    }
    catch (e) {
      if (run === scanRun) scanError.value = message(e, 'Couldn\'t list the keys.')
    }
    finally {
      if (run === scanRun) scanning.value = false
    }
  }

  function toggleFolder(path: string) {
    const next = new Set(openFolders.value)
    if (next.has(path)) next.delete(path)
    else next.add(path)
    openFolders.value = next
  }

  function closeKey() {
    valueRun++
    selectedKey.value = null
    keyInfo.value = null
    value.value = null
    valueNext.value = null
    valueError.value = null
    valueLoading.value = false
  }

  /** Adds one page from Rust to what is loaded. The first page decides the shape. */
  function append(page: RedisValuePage_Serialize, first: boolean) {
    const v = value.value
    switch (page.type) {
      case 'string': value.value = { type: 'string', value: page.value, totalBytes: page.total_bytes ?? 0, truncated: page.truncated }; valueNext.value = null; break
      case 'json': value.value = { type: 'json', text: page.text }; valueNext.value = null; break
      case 'unsupported': value.value = { type: 'unsupported', keyType: page.key_type }; valueNext.value = null; break
      case 'hash': value.value = { type: 'hash', fields: [...(!first && v?.type === 'hash' ? v.fields : []), ...page.fields] }; valueNext.value = page.next; break
      case 'list': value.value = { type: 'list', items: [...(!first && v?.type === 'list' ? v.items : []), ...page.items] }; valueNext.value = page.next; break
      case 'set': value.value = { type: 'set', members: [...(!first && v?.type === 'set' ? v.members : []), ...page.members] }; valueNext.value = page.next; break
      case 'zset': value.value = { type: 'zset', members: [...(!first && v?.type === 'zset' ? v.members : []), ...page.members] }; valueNext.value = page.next; break
      case 'stream': value.value = { type: 'stream', entries: [...(!first && v?.type === 'stream' ? v.entries : []), ...page.entries] }; valueNext.value = page.next; break
    }
  }

  async function loadValue(key: RedisBytes, cursor: string, full: boolean) {
    const run = valueRun
    valueLoading.value = true
    valueError.value = null
    try {
      const page = await api.redisValuePage(connId.value, key, cursor, full)
      if (run === valueRun) append(page, cursor === '')
    }
    catch (e) {
      if (run === valueRun) valueError.value = { code: (e as { code?: string }).code ?? 'internal', message: message(e, 'Couldn\'t read the value.') }
    }
    finally {
      if (run === valueRun) valueLoading.value = false
    }
  }

  /** Opens a key: its header details and the first page of its value. */
  async function openKey(key: RedisBytes) {
    if (!connId.value || status.value !== 'ready') return
    closeKey()
    const run = valueRun
    selectedKey.value = key
    if (key.kind === 'text' && db.value) openFolders.value = new Set([...openFolders.value, ...foldersOf(key.text, db.value.separator)])
    valueLoading.value = true
    try {
      const info = await api.redisKeyInfo(connId.value, key)
      if (run !== valueRun) return
      keyInfo.value = info
      if (info.key_type === 'none') {
        valueLoading.value = false
        valueError.value = { code: 'not_found', message: 'This key is gone. It expired or was deleted.' }
        return
      }
    }
    catch (e) {
      if (run !== valueRun) return
      valueLoading.value = false
      valueError.value = { code: (e as { code?: string }).code ?? 'internal', message: message(e, 'Couldn\'t read the key.') }
      return
    }
    await loadValue(key, '', false)
  }

  /** "Load more": the next 500 items. */
  const loadMore = () => (selectedKey.value && valueNext.value && !valueLoading.value ? loadValue(selectedKey.value, valueNext.value, false) : Promise.resolve())
  /** "Load all" on a string that opened as a preview. */
  const loadAll = () => (selectedKey.value && !valueLoading.value ? loadValue(selectedKey.value, '', true) : Promise.resolve())
  /** "Reload": the open key again, from the server. */
  const reloadKey = () => (selectedKey.value ? openKey(selectedKey.value) : Promise.resolve())

  // ---- Writes (F15). Every change goes to Rust's gate; this side only shows what the gate answers.
  /** A write the gate wants confirmed: what Rust says it will do, and the editor's wording for it. */
  const pendingWrite = ref<{ pending: RedisPending_Serialize, ask: WriteAsk, op: RedisWriteOp_Deserialize | null, entry?: number, after?: () => void } | null>(null)
  const writeBusy = ref(false)
  const sameKey = (a: RedisBytes, b: RedisBytes) => bytesId(a) === bytesId(b)

  /** The open key again, without clearing what is shown (after a change). A key whose last item went is gone. */
  async function refreshKey() {
    const key = selectedKey.value
    if (!key || !connId.value) return
    const run = ++valueRun
    try {
      const info = await api.redisKeyInfo(connId.value, key)
      if (run !== valueRun) return
      if (info.key_type === 'none') {
        keyRows.value = keyRows.value.filter(r => !sameKey(r.key, key))
        closeKey()
        toast('That was its last item, so the key is gone.')
        return
      }
      keyInfo.value = info
      keyRows.value = keyRows.value.map(r => (sameKey(r.key, key) ? { ...r, key_type: info.key_type, ttl_ms: info.ttl_ms } : r))
      const v = value.value
      const page = await api.redisValuePage(connId.value, key, '', v?.type === 'string' && !v.truncated)
      if (run === valueRun) append(page, true)
    }
    catch (e) {
      if (run === valueRun) valueError.value = { code: (e as { code?: string }).code ?? 'internal', message: message(e, 'Couldn\'t read the key.') }
    }
  }

  /** What the screen needs after a write went through. */
  async function afterWrite(op: RedisWriteOp_Deserialize) {
    const bump = (by: number) => { keys.value = { ...keys.value, [selected.value]: Math.max(0, (keys.value[selected.value] ?? 0) + by) } }
    switch (op.op) {
      case 'delete':
        keyRows.value = keyRows.value.filter(r => !sameKey(r.key, op.key))
        bump(-1)
        closeKey()
        toast('Key deleted.')
        break
      case 'rename':
        keyRows.value = keyRows.value.map(r => (sameKey(r.key, op.key) ? { ...r, key: op.to } : r))
        toast('Key renamed.')
        await openKey(op.to)
        break
      case 'create':
        keyRows.value = [{ key: op.key, key_type: op.key_type, ttl_ms: op.ttl_ms }, ...keyRows.value.filter(r => !sameKey(r.key, op.key))]
        bump(1)
        toast('Key created.')
        await openKey(op.key)
        break
      case 'publish':
        toast('Published.')
        break
      default:
        await refreshKey()
    }
  }

  /** Asks Rust to make a change. True when it ran; false when it was refused, failed, or now waits in a dialog. */
  async function write(op: RedisWriteOp_Deserialize, ask: WriteAsk): Promise<boolean> {
    if (!connId.value || status.value !== 'ready') return false
    try {
      const out = await api.redisWritePrepare(connId.value, op)
      if (out.status === 'pending') {
        pendingWrite.value = { pending: out.pending, ask, op }
        return false
      }
      await afterWrite(op)
      return true
    }
    catch (e) {
      toast.error(message(e, 'Couldn\'t make that change.'))
      return false
    }
  }

  /** The dialog's button: runs the prepared write. Rust uses a confirmation up even when it refuses, so the dialog closes either way. */
  async function confirmWrite(typedName: string | null = null) {
    const p = pendingWrite.value
    if (!p || writeBusy.value) return
    writeBusy.value = true
    try {
      if (p.op) {
        await api.redisWriteCommit(connId.value, p.pending.id, typedName)
        pendingWrite.value = null
        await afterWrite(p.op)
      }
      else {
        // A dangerous console command: the reply belongs in the console's output.
        const reply = await api.redisConsoleCommit(connId.value, p.pending.id, typedName)
        pendingWrite.value = null
        const e = consoleEntries.value.find(x => x.id === p.entry)
        if (e) fill(e, reply)
        p.after?.()
      }
    }
    catch (e) {
      pendingWrite.value = null
      const entry = consoleEntries.value.find(x => x.id === p.entry)
      if (entry) Object.assign(entry, { state: 'refused', lines: [message(e, 'Not sent.')] })
      else toast.error(message(e, 'Couldn\'t make that change.'))
    }
    finally {
      writeBusy.value = false
    }
  }
  function cancelWrite() {
    if (writeBusy.value) return
    const e = consoleEntries.value.find(x => x.id === pendingWrite.value?.entry)
    if (e) Object.assign(e, { state: 'cancelled', lines: ['Cancelled. Nothing was sent.'] })
    pendingWrite.value = null
  }

  // ---- Console (F16). Rust parses the line, asks the gate and prints the reply; this side keeps the output.
  const consoleEntries = ref<ConsoleEntry[]>([])
  const consoleHistory = ref<string[]>([])
  const commandList = ref<RedisCommandInfo_Serialize[]>([])
  let entrySeq = 0

  function fill(e: ConsoleEntry, reply: RedisConsoleReply_Serialize) {
    Object.assign(e, { state: 'done', lines: reply.lines, error: reply.error, truncated: reply.truncated, elapsedMs: reply.elapsed_ms, ask: null })
    if (reply.selected !== null && !reply.error) {
      // SELECT ran in the console: the whole tab is on that logical database now.
      selected.value = reply.selected
      void scan()
    }
  }

  /** History and the command list, once per connection (the Console view asks when it first shows). */
  async function loadConsole() {
    if (!connId.value) return
    consoleHistory.value = await api.redisConsoleHistory(dbId.value).catch(() => [])
    if (!commandList.value.length) commandList.value = await api.redisCommands(connId.value).catch(() => [])
  }

  /** Runs one line. `title` and `after` are for the Server view's "Clear log…", which is a console command too. */
  async function consoleRun(line: string, dangerTitle?: string, after?: () => void) {
    const text = line.trim()
    if (!text || !connId.value || status.value !== 'ready') return
    consoleEntries.value.push({ id: ++entrySeq, prompt: `${selected.value}>`, line: text, state: 'running', lines: [], error: false, truncated: false, elapsedMs: null, showAll: false, ask: null })
    const e = consoleEntries.value[consoleEntries.value.length - 1]!
    if (consoleHistory.value[0] !== text) consoleHistory.value = [text, ...consoleHistory.value.filter(h => h !== text)].slice(0, 500)
    try {
      const out = await api.redisConsoleExec(connId.value, text)
      if (out.status === 'reply') {
        fill(e, out.reply)
        after?.()
      }
      else if (out.pending.level === 'typed') {
        e.lines = ['Waiting for you to type the database\'s name.']
        const name = text.split(/\s+/).slice(0, 2).join(' ').toUpperCase()
        pendingWrite.value = { pending: out.pending, ask: { title: dangerTitle ?? `Run ${name}?`, action: dangerTitle ? dangerTitle.replace(/\?$/, '') : `Run ${name}`, danger: true, text: dangerText(text) }, op: null, entry: e.id, after }
      }
      else Object.assign(e, { state: 'pending', ask: { pendingId: out.pending.id, heavy: out.heavy, safer: out.safer } })
    }
    catch (err) {
      Object.assign(e, { state: 'refused', lines: [message(err, 'Not sent.')] })
    }
  }
  /** What a dangerous console command destroys, in a sentence. */
  function dangerText(line: string) {
    const name = line.trim().split(/\s+/)[0]!.toUpperCase()
    const n = (keys.value[selected.value] ?? 0).toLocaleString('en-US')
    if (name === 'FLUSHDB') return `This removes all ${n} keys in db${selected.value}. It can't be undone, and everyone using this database is affected.`
    if (name === 'FLUSHALL') return 'This removes every key in every logical database. It can\'t be undone, and everyone using this server is affected.'
    return 'This can change how the server runs for everyone using it, and may not be undoable.'
  }
  /** "Send" or "Run anyway…" on a held console command. */
  async function consoleSend(id: number) {
    const e = consoleEntries.value.find(x => x.id === id)
    if (!e?.ask) return
    const pendingId = e.ask.pendingId
    Object.assign(e, { state: 'running', ask: null })
    try {
      fill(e, await api.redisConsoleCommit(connId.value, pendingId, null))
    }
    catch (err) {
      Object.assign(e, { state: 'refused', lines: [message(err, 'Not sent.')] })
    }
  }
  function consoleCancel(id: number) {
    const e = consoleEntries.value.find(x => x.id === id)
    if (e) Object.assign(e, { state: 'cancelled', lines: ['Cancelled. Nothing was sent.'], ask: null })
  }
  const consoleClear = () => { consoleEntries.value = [] }
  async function consoleClearHistory() {
    await api.redisConsoleHistoryClear(dbId.value).catch(() => {})
    consoleHistory.value = []
  }

  // ---- Pub/Sub (F17). Rust listens on its own connection and sends batches; this side keeps the last 5,000.
  const subs = ref<RedisSub[]>([])
  // shallowRef: a batch replaces the array, so Vue never walks 5,000 messages to make them reactive.
  const messages = shallowRef<RedisMessage_Serialize[]>([])
  const heldMessages = shallowRef<RedisMessage_Serialize[]>([])
  const received = ref(0)
  const lost = ref(0)
  const paused = ref(false)
  const subError = ref('')
  const isPattern = (name: string) => /[*?[]/.test(name)

  function onSubEvent(e: RedisSubEvent_Serialize) {
    if (e.type === 'ended') {
      subs.value = subs.value.map(x => ({ ...x, live: false }))
      return
    }
    received.value += e.messages.length + e.dropped
    lost.value += e.dropped
    const counts = new Map<string, number>()
    for (const m of e.messages) {
      const key = m.pattern === null ? `c:${bytesText(m.channel)}` : `p:${m.pattern}`
      counts.set(key, (counts.get(key) ?? 0) + 1)
    }
    subs.value = subs.value.map(x => ({ ...x, count: x.count + (counts.get(`${x.pattern ? 'p' : 'c'}:${x.name}`) ?? 0) }))
    // Newest first. While paused they wait; whatever falls off the end of either list is counted as lost.
    const target = paused.value ? heldMessages : messages
    const next = [...e.messages].reverse().concat(target.value)
    if (next.length > KEEP_MESSAGES) {
      if (paused.value) lost.value += next.length - KEEP_MESSAGES
      next.length = KEEP_MESSAGES
    }
    target.value = next
  }

  /** Starts listening to a channel, or to a pattern when the name has `*`, `?` or `[` in it. */
  async function listen(raw: string) {
    const name = raw.trim()
    subError.value = ''
    if (!name || !connId.value || status.value !== 'ready') return false
    const pattern = isPattern(name)
    if (subs.value.some(x => x.name === name && x.pattern === pattern)) return true
    try {
      await api.redisSubStart(connId.value, textBytes(name), pattern, onSubEvent)
      subs.value = [...subs.value, { name, pattern, count: 0, live: true }]
      return true
    }
    catch (e) {
      subError.value = message(e, 'Couldn\'t start listening.')
      return false
    }
  }
  async function unlisten(sub: RedisSub) {
    subs.value = subs.value.filter(x => x !== sub && !(x.name === sub.name && x.pattern === sub.pattern))
    if (connId.value) await api.redisSubStop(connId.value, textBytes(sub.name), sub.pattern).catch(() => {})
  }
  /** After a reconnect the listening connection is new: listen to everything again. */
  async function relisten() {
    for (const x of subs.value) {
      const ok = await api.redisSubStart(connId.value, textBytes(x.name), x.pattern, onSubEvent).then(() => true, () => false)
      subs.value = subs.value.map(y => (y.name === x.name && y.pattern === x.pattern ? { ...y, live: ok } : y))
    }
  }
  function setPaused(on: boolean) {
    paused.value = on
    if (!on && heldMessages.value.length) {
      messages.value = heldMessages.value.concat(messages.value).slice(0, KEEP_MESSAGES)
      heldMessages.value = []
    }
  }
  function clearMessages() {
    messages.value = []
    heldMessages.value = []
    received.value = 0
    lost.value = 0
  }
  const publish = (channel: string, text: string) => write({ op: 'publish', channel: textBytes(channel), message: textBytes(text) }, { title: 'Publish this message?', action: 'Publish' })

  // ---- Bulk actions (F18). Rust scans, keeps the matching keys and runs on exactly those; this side shows progress.
  const bulkPreview = (pattern: string, keyType: string | null, onEvent: (e: RedisBulkEvent) => void) => api.redisBulkPreview(connId.value, pattern, keyType, onEvent)
  async function bulkRun(previewId: string, action: RedisBulkAction, typedName: string, onEvent: (e: RedisBulkEvent) => void) {
    const result = await api.redisBulkRun(connId.value, previewId, action, typedName, onEvent)
    // The tree and the key count show what is left (SELECT on the same database answers with its size).
    const k = await api.redisSelect(connId.value, selected.value).catch(() => null)
    if (k) keys.value = { ...keys.value, [k.index]: k.keys ?? 0 }
    void scan()
    return result
  }
  const bulkStop = () => { if (connId.value) void api.redisBulkStop(connId.value) }

  // ---- Server view (S31): INFO and the slow log, read again while the view shows.
  const stats = ref<RedisServerStats | null>(null)
  const slowlog = ref<RedisSlowEntry[]>([])
  const serverAt = ref<number | null>(null)
  const serverError = ref('')
  const serverLoading = ref(false)
  async function refreshServer() {
    if (!connId.value || status.value !== 'ready' || serverLoading.value) return
    serverLoading.value = true
    try {
      const [info, slow] = await Promise.all([api.redisInfo(connId.value), api.redisSlowlog(connId.value)])
      stats.value = info
      slowlog.value = slow
      serverAt.value = Date.now()
      serverError.value = ''
      keys.value = Object.fromEntries(info.databases.map(d => [d.index, d.keys ?? 0]))
    }
    catch (e) {
      serverError.value = message(e, 'Couldn\'t read the server\'s details.')
    }
    finally {
      serverLoading.value = false
    }
  }

  /** Closes the connection (tab closed, or the vault locked). */
  function close() {
    generation++
    if (connId.value) void api.redisClose(connId.value)
    connId.value = ''
    status.value = 'closed'
    steps.value = []
    prompt.value = null
    info.value = null
    dropped.value = null
    scanRun++
    keyRows.value = []
    scanCursor.value = ''
    scanned.value = 0
    pendingWrite.value = null
    consoleEntries.value = []
    commandList.value = []
    stats.value = null
    slowlog.value = []
    subs.value = []
    messages.value = []
    heldMessages.value = []
    closeKey()
  }

  return { dbId, connId, status, steps, prompt, info, error, dropped, latencyMs, selected, keys, view, open, retry, respond, select, close,
    pattern, typeFilter, keyRows, scanCursor, scanned, scanning, scanError, scanDone, openFolders, selectedKey, selectedId, keyInfo, value, valueNext, valueLoading, valueError,
    setSeparator, scan, scanMore, toggleFolder, openKey, closeKey, loadMore, loadAll, reloadKey,
    pendingWrite, writeBusy, write, confirmWrite, cancelWrite,
    consoleEntries, consoleHistory, commandList, loadConsole, consoleRun, consoleSend, consoleCancel, consoleClear, consoleClearHistory,
    stats, slowlog, serverAt, serverError, serverLoading, refreshServer,
    subs, messages, heldMessages, received, lost, paused, subError, listen, unlisten, setPaused, clearMessages, publish, bulkPreview, bulkRun, bulkStop }
}

// One store per Redis tab, like the SFTP tabs (P30): each tab has its own connection, steps and view.
const makeStore = (key: string) => defineStore(`redis:${key}`, setup)
const tabStores = new Map<string, ReturnType<typeof makeStore>>()

export function useRedisStore(tabKey: string) {
  if (!tabStores.has(tabKey)) tabStores.set(tabKey, makeStore(tabKey))
  return tabStores.get(tabKey)!()
}

/** A closed tab's store is forgotten, so a later tab with a new key starts clean. */
export function dropRedisStore(tabKey: string) {
  if (!tabStores.has(tabKey)) return
  useRedisStore(tabKey).close()
  tabStores.delete(tabKey)
}

/** Vault locked: Rust already closed every connection; forget the tabs' state. */
export function clearAllRedis() {
  for (const key of [...tabStores.keys()]) dropRedisStore(key)
}
