// The fake's Pub/Sub and bulk actions (replica/redis/spec.md, R5). A test publishes with `fake.redisPublish`;
// a publish through the write gate reaches the listeners too. Bulk: preview keeps the matching keys, the run acts
// on exactly those in batches of 500 with a pause, and Stop ends it after the current batch.
import { fail, type Handlers } from '../types'
import type { Model } from '../model'
import type { RedisBytes } from '../../../app/types/bindings'
import { globToRegExp, keyspace } from './redisKeys'

const show = (b: RedisBytes) => (b.kind === 'text' ? b.text : '')
const text = (s: string): RedisBytes => ({ kind: 'text', text: s })

/** Sends a message to every tab that listens to the channel, by name or by pattern. Returns how many got it. */
export function deliver(m: Model, send: (channel: number, message: unknown) => void, channel: string, payload: string, times = 1): number {
  let receivers = 0
  for (const sub of m.redisSubs) {
    if (!(sub.pattern ? globToRegExp(sub.name).test(channel) : sub.name === channel)) continue
    receivers++
    const messages = Array.from({ length: times }, (_, i) => ({ at_ms: m.now + i, channel: text(channel), pattern: sub.pattern ? sub.name : null, payload: text(times > 1 ? `${payload} ${i + 1}` : payload) }))
    // Like Rust: at most 500 a batch; the rest of a flood is counted, not sent.
    for (let at = 0; at < messages.length; at += 2000) {
      const window = messages.slice(at, at + 2000)
      send(sub.events, { type: 'messages', messages: window.slice(0, 500), dropped: Math.max(0, window.length - 500) })
    }
  }
  return receivers
}

function database(m: Model, connId: string) {
  const c = m.redisConns.get(connId)
  if (!c || c.closed) fail('not_found', 'That Redis connection is closed.')
  return m.redisDbs.find(d => d.id === c.db) ?? fail('not_found', 'That database is no longer saved.')
}
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))

type Keys = 'redisSubStart' | 'redisSubStop' | 'redisBulkPreview' | 'redisBulkRun' | 'redisBulkStop'
export const redisPubSub: Pick<Handlers, Keys> = {
  redisSubStart: (a, { m, channel, tag }) => {
    database(m, a.connId)
    const name = show(a.name)
    if (!name) fail('validation', 'Type a channel name or a pattern such as orders.*.')
    if (name === 'old-server') fail('validation', 'Listening needs Redis 6 or newer (this server speaks the older protocol).')
    m.redisSubs = m.redisSubs.filter(s => !(s.connId === a.connId && s.name === name && s.pattern === a.pattern))
    m.redisSubs.push({ connId: a.connId, name, pattern: !!a.pattern, events: channel(a.messages) })
    tag(`redis_sub:${a.pattern ? 'p' : 'c'}:${name}`)
    return null
  },
  redisSubStop: (a, { m, tag }) => {
    const name = show(a.name)
    m.redisSubs = m.redisSubs.filter(s => !(s.connId === a.connId && s.name === name && s.pattern === !!a.pattern))
    tag(`redis_unsub:${name}`)
    return null
  },
  redisBulkPreview: async (a, { m, id, channel, send }) => {
    database(m, a.connId)
    if (!m.redisKeys.length) m.redisKeys = keyspace()
    const events = channel(a.progress)
    const match = a.pattern && a.pattern !== '*' ? globToRegExp(a.pattern) : null
    m.redisBulkStop = false
    const found: string[] = []
    const names = m.redisKeys.filter(k => !a.keyType || k.value.type === a.keyType).map(k => (k.key.kind === 'text' ? k.key.text : ''))
    // Walk in pages, so a test can press Stop in between.
    for (let at = 0; at < names.length; at += 500) {
      if (m.redisBulkStop) return { id: '', count: found.length, sample: found.slice(0, 20).map(text), complete: false }
      found.push(...names.slice(at, at + 500).filter(n => n && (!match || match.test(n))))
      send(events, { type: 'scanning', found: found.length, scanned: at + 500 })
      if (m.redisBulkSlow) await sleep(60)
    }
    const previewId = found.length ? id('rbulk') : ''
    if (previewId) m.redisBulk = { id: previewId, connId: a.connId, keys: found }
    return { id: previewId, count: found.length, sample: found.slice(0, 20).map(text), complete: true }
  },
  redisBulkRun: async (a, { m, channel, send, tag }) => {
    const db = database(m, a.connId)
    if (db.write_mode === 'read_only') {
      tag('redis_refused:bulk')
      fail('redis_read_only_mode', `${db.name} is read-only in Dalan. Nothing was sent.`)
    }
    if (a.typedName !== db.name) fail('redis_confirm_needed', 'The name doesn\'t match the database\'s name. Nothing was sent.')
    const held = m.redisBulk
    m.redisBulk = null
    if (!held || held.id !== a.previewId || held.connId !== a.connId) fail('redis_confirm_needed', 'That preview is gone or too old. Nothing was sent. Preview again.')
    const events = channel(a.progress)
    m.redisBulkStop = false
    tag(`redis_write:bulk_${a.action.action}`)
    let processed = 0
    let changed = 0
    for (let at = 0; at < held.keys.length; at += 500) {
      if (m.redisBulkStop) break
      const batch = new Set(held.keys.slice(at, at + 500))
      for (const k of [...m.redisKeys]) {
        if (k.key.kind !== 'text' || !batch.has(k.key.text)) continue
        if (a.action.action === 'delete') m.redisKeys.splice(m.redisKeys.indexOf(k), 1)
        else if (a.action.action === 'expire') k.ttl_ms = a.action.ttl_ms
        else if (k.ttl_ms === null) continue
        else k.ttl_ms = null
        changed++
      }
      processed += batch.size
      send(events, { type: 'running', done: processed, total: held.keys.length })
      if (m.redisBulkSlow) await sleep(60)
    }
    return { processed, changed, stopped: processed < held.keys.length }
  },
  redisBulkStop: (_a, { m }) => {
    m.redisBulkStop = true
  },
}
