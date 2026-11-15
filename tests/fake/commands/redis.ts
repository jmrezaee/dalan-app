// Redis (replica/redis/spec.md): saved databases and a tab's connection. The fake holds the databases like Rust's
// table would (a password is only a flag) and plays a connection's events: by itself (plan, steps, ready) unless a
// test took over with `fake.redisManual()` and sends the events with `fake.redisEvent(connId, e)`.
import { fail, type Handlers } from '../types'
import type { RedisDb } from '../model'

type Keys = 'redisDbList' | 'redisDbSave' | 'redisDbDelete' | 'redisDbTest' | 'redisOpen' | 'redisRespond' | 'redisSelect' | 'redisClose'

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
    m.redisConns.set(connId, { events, db: d.id, closed: false })
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
        send(events, { type: 'ready', info: { version: '7.2.4', mode: 'standalone', replica: false, resp3: true, db_count: 16, databases: [{ index: 0, keys: 12408 }, { index: 3, keys: 2 }], selected: d.default_db, latency_ms: 3 } })
      })
    }
    return { conn_id: connId }
  },
  redisRespond: (a, { m }) => {
    m.responses.push({ id: a.connId, prompt: a.promptId, answer: a.answer })
    return null
  },
  redisSelect: (a, { m }) => {
    if (!m.redisConns.has(a.connId)) fail('not_found', 'That Redis connection is closed.')
    return { index: a.index, keys: a.index === 0 ? 12408 : a.index === 3 ? 2 : 0 }
  },
  redisClose: (a, { m, tag }) => {
    const c = m.redisConns.get(a.connId)
    if (c) c.closed = true
    tag(`redis_close:${a.connId}`)
  },
}
