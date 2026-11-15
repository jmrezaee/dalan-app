import { defineStore } from 'pinia'
import type { RedisBytes, RedisDbKeys, RedisKeyInfo_Serialize, RedisKeyRow, RedisPlanStep_Serialize, PromptKind, RedisEvent_Serialize, RedisServerInfo, RedisStepState, RedisValuePage_Serialize } from '~/types/bindings'
import { bytesId } from '~/utils/redisBytes'
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
        error.value = null
        dropped.value = null
        prompt.value = null
        break
      case 'latency':
        latencyMs.value = e.ms
        break
      case 'dropped':
        status.value = 'dropped'
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
    closeKey()
  }

  return { dbId, connId, status, steps, prompt, info, error, dropped, latencyMs, selected, keys, view, open, retry, respond, select, close,
    pattern, typeFilter, keyRows, scanCursor, scanned, scanning, scanError, scanDone, openFolders, selectedKey, selectedId, keyInfo, value, valueNext, valueLoading, valueError,
    setSeparator, scan, scanMore, toggleFolder, openKey, closeKey, loadMore, loadAll, reloadKey }
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
