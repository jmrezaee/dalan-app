// Redis (replica/redis/spec.md): saved databases and a tab's connection. The fake holds the databases like Rust's
// table would (a password is only a flag) and plays a connection's events: by itself (plan, steps, ready) unless a
// test took over with `fake.redisManual()` and sends the events with `fake.redisEvent(connId, e)`.
import { fail, type Handlers } from '../types'
import type { Model, RedisDb } from '../model'
import type { RedisBytes } from '../../../app/types/bindings'
import { globToRegExp, keyspace, otherKeyspace, type FakeKey } from './redisKeys'

type Keys = 'redisDbList' | 'redisDbSave' | 'redisDbDelete' | 'redisDbTest' | 'redisOpen' | 'redisRespond' | 'redisSelect' | 'redisClose'
  | 'redisScan' | 'redisKeyInfo' | 'redisValuePage'

const PAGE = 500
const id = (b: RedisBytes) => (b.kind === 'text' ? `t:${b.text}` : `b:${b.bytes.join(',')}`)
const show = (b: RedisBytes) => (b.kind === 'text' ? b.text : b.bytes.map(n => `\\x${n.toString(16).padStart(2, '0')}`).join(''))
const text = (s: string): RedisBytes => ({ kind: 'text', text: s })
/** The server's word for a type (`string`, `hash`, ...; the JSON module's keys are `json` to the UI). */
const typeOf = (k: FakeKey) => k.value.type

/** The keys of the logical database this connection is on. db0 is built on first use. */
function keysOf(m: Model, connId: string): FakeKey[] {
  const c = m.redisConns.get(connId)
  if (!c || c.closed) fail('not_found', 'That Redis connection is closed.')
  if (c.index !== 0) return otherKeyspace(c.index)
  if (!m.redisKeys.length) m.redisKeys = keyspace()
  return m.redisKeys
}
const length = (k: FakeKey) => {
  const v = k.value
  switch (v.type) {
    case 'string': return v.totalBytes ?? (v.value.kind === 'text' ? new TextEncoder().encode(v.value.text).length : v.value.bytes.length)
    case 'hash': return v.fields.length
    case 'list': return v.items.length
    case 'set': return v.members.length
    case 'zset': return v.members.length
    case 'stream': return v.entries.length
    default: return 0
  }
}

/** What the UI gets: the row with `has_password`, never the password. */
const summary = (m: import('../model').Model, d: RedisDb) => ({
  id: d.id, name: d.name, environment: d.environment, route: d.route, host_id: d.host_id,
  host_label: m.hosts.find(h => h.id === d.host_id)?.label ?? null,
  address: d.address, port: d.port, kind: d.kind, sentinel_master: d.sentinel_master, username: d.username,
  has_password: d.password, tls: d.tls, tls_server_name: d.tls_server_name, write_mode: d.write_mode,
  default_db: d.default_db, key_separator: d.key_separator, last_opened_at: d.last_opened_at,
})

function check(m: import('../model').Model, i: any) {
  if (!String(i.name ?? '').trim()) fail('validation', 'Give the database a name of up to 64 characters.')
  if (!String(i.address ?? '').trim() || /\s|\//.test(String(i.address).trim())) fail('validation', 'The address is a host name or an IP address, without spaces or a path.')
  if (!(Number(i.port) >= 1 && Number(i.port) <= 65535)) fail('validation', 'The port must be between 1 and 65535.')
  if (i.route === 'host' && !i.host_id) fail('validation', 'Pick the host to go through.')
  if (i.route === 'host' && !m.hosts.some(h => h.id === i.host_id)) fail('validation', 'That host is gone. Pick another one.')
}

export const redis: Pick<Handlers, Keys> = {
  redisDbList: (_a, { m }) => [...m.redisDbs].sort((a, b) => a.name.localeCompare(b.name)).map(d => summary(m, d)),
  redisDbSave: (a, { m, id }) => {
    const i = a.input
    check(m, i)
    const existing = i.id ? m.redisDbs.find(d => d.id === i.id) : undefined
    if (i.id && !existing) fail('not_found', 'That database is no longer saved.')
    const password = i.password ? true : i.clear_password ? false : (existing?.password ?? false)
    const d: RedisDb = {
      id: existing?.id ?? id('rdb'), name: String(i.name).trim(), environment: i.environment, route: i.route, host_id: i.route === 'host' ? i.host_id : null,
      address: String(i.address).trim(), port: Number(i.port), kind: i.kind, sentinel_master: i.sentinel_master ?? null, username: i.username || null, password,
      tls: !!i.tls, tls_server_name: i.tls_server_name || null, write_mode: i.write_mode, default_db: i.default_db ?? 0, key_separator: i.key_separator ?? ':',
      last_opened_at: existing?.last_opened_at ?? null,
    }
    // Like Rust: the editor can't loosen a production database's mode (that needs the master password).
    const rank = { read_only: 2, confirm: 1, open: 0 } as const
    if (existing?.environment === 'production' && rank[d.write_mode] < rank[existing.write_mode]) fail('validation', 'A production database\'s write mode changes from the mode button in its tab, with the master password.')
    if (existing) Object.assign(existing, d)
    else m.redisDbs.push(d)
    return summary(m, d)
  },
  redisDbDelete: (a, { m }) => {
    const i = m.redisDbs.findIndex(d => d.id === a.id)
    if (i < 0) fail('not_found', 'That database is no longer saved.')
    m.redisDbs.splice(i, 1)
    return null
  },
  redisDbTest: (a, { m }) => {
    check(m, a.input)
    return { version: '7.2.4', mode: 'standalone', replica: false, latency_ms: 3 }
  },
  redisOpen: (a, { m, id, channel, send }) => {
    const d = m.redisDbs.find(x => x.id === a.dbId)
    if (!d) fail('not_found', 'That database is no longer saved.')
    const connId = id('rconn')
    const events = channel(a.events)
    m.redisConns.set(connId, { events, db: d.id, closed: false, index: d.default_db })
    if (!m.redisKeys.length) m.redisKeys = keyspace()
    const keysInDb0 = m.redisKeys.length
    d.last_opened_at = m.now
    if (!m.redisManual) {
      // What a quick, healthy connection sends. Through a host there is one sign-in step per hop first.
      const host = m.hosts.find(h => h.id === d.host_id)
      const steps = [
        ...(host ? [{ label: `Sign in to ${host.label}`, detail: null }] : []),
        { label: `Reach Redis at ${d.address}:${d.port}`, detail: d.tls ? 'with TLS' : null },
        { label: d.username ? `Sign in as ${d.username}` : 'Sign in to Redis', detail: null },
      ]
      queueMicrotask(() => {
        send(events, { type: 'plan', steps })
        steps.forEach((_s, index) => {
          send(events, { type: 'step', index, state: 'active', detail: null })
          send(events, { type: 'step', index, state: 'done', detail: null })
        })
        send(events, { type: 'ready', info: { version: '7.2.4', mode: 'standalone', replica: false, resp3: true, db_count: 16, databases: [{ index: 0, keys: keysInDb0 }, { index: 3, keys: 2 }], selected: d.default_db, latency_ms: 3 } })
      })
    }
    return { conn_id: connId }
  },
  redisRespond: (a, { m }) => {
    m.responses.push({ id: a.connId, prompt: a.promptId, answer: a.answer })
    return null
  },
  redisSelect: (a, { m }) => {
    const c = m.redisConns.get(a.connId)
    if (!c) fail('not_found', 'That Redis connection is closed.')
    c.index = a.index
    return { index: a.index, keys: keysOf(m, a.connId).length }
  },
  // SCAN: the cursor is a position in the keyspace; one call looks at `count` keys and returns those that match,
  // so a narrow pattern can come back empty with more to scan, like the real thing.
  redisScan: (a, { m }) => {
    const all = keysOf(m, a.connId)
    const start = Number(a.cursor || 0)
    const window = all.slice(start, start + a.count)
    const match = a.pattern && a.pattern !== '*' ? globToRegExp(a.pattern) : null
    const keys = window
      .filter(k => (!match || match.test(show(k.key))) && (!a.keyType || typeOf(k) === a.keyType))
      .map(k => ({ key: k.key, key_type: typeOf(k), ttl_ms: k.ttl_ms }))
    return { cursor: start + a.count < all.length ? String(start + a.count) : '0', keys }
  },
  redisKeyInfo: (a, { m }) => {
    const k = keysOf(m, a.connId).find(x => id(x.key) === id(a.key))
    if (!k) return { key: a.key, key_type: 'none', ttl_ms: null, length: 0, memory_bytes: null, encoding: null, stream_groups: [] }
    return { key: k.key, key_type: typeOf(k), ttl_ms: k.ttl_ms, length: length(k), memory_bytes: typeOf(k) === 'hash' ? 412 : null, encoding: null, stream_groups: k.value.type === 'stream' ? k.value.groups : [] }
  },
  redisValuePage: (a, { m }) => {
    const k = keysOf(m, a.connId).find(x => id(x.key) === id(a.key))
    if (!k) fail('not_found', 'That key is gone.')
    const v = k.value
    const start = Number(a.cursor || 0)
    const page = <T>(items: T[]) => ({ items: items.slice(start, start + PAGE), next: start + PAGE < items.length ? String(start + PAGE) : null })
    switch (v.type) {
      case 'string': {
        const big = v.totalBytes !== undefined
        return { type: 'string', value: big && a.full ? v.full! : v.value, total_bytes: length(k), truncated: big && !a.full }
      }
      case 'json': return { type: 'json', text: v.text }
      case 'hash': { const p = page(v.fields); return { type: 'hash', fields: p.items.map(([f, val]) => ({ field: text(f), value: text(val) })), next: p.next } }
      case 'list': { const p = page(v.items); return { type: 'list', start, items: p.items.map(text), next: p.next } }
      case 'set': { const p = page(v.members); return { type: 'set', members: p.items.map(text), next: p.next } }
      case 'zset': { const p = page(v.members); return { type: 'zset', members: p.items.map(([member, score]) => ({ member: text(member), score: String(score) })), next: p.next } }
      case 'stream': return { type: 'stream', entries: v.entries.map(e => ({ id: e.id, fields: e.fields.map(([f, val]) => ({ field: text(f), value: text(val) })) })), next: null }
      default: return { type: 'unsupported', key_type: v.type }
    }
  },
  redisClose: (a, { m, tag }) => {
    const c = m.redisConns.get(a.connId)
    if (c) c.closed = true
    tag(`redis_close:${a.connId}`)
  },
}
