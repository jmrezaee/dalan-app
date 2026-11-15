import { defineStore } from 'pinia'
import type { RedisDbKeys, RedisPlanStep_Serialize, PromptKind, RedisEvent_Serialize, RedisServerInfo, RedisStepState } from '~/types/bindings'
import { useTauri } from '~/composables/useTauri'

export type RedisStatus = 'connecting' | 'ready' | 'dropped' | 'failed' | 'closed'
/** `note` is the step's fixed remark ("through bastion"); `detail` is what shows now: a progress line, why it failed, or the note. */
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
      if (gen === generation) connId.value = opened.conn_id
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
    }
    catch (e) {
      error.value = { code: (e as { code?: string }).code ?? 'internal', message: e instanceof Error ? e.message : 'Couldn\'t switch the database.' }
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
  }

  return { dbId, connId, status, steps, prompt, info, error, dropped, latencyMs, selected, keys, view, open, retry, respond, select, close }
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
