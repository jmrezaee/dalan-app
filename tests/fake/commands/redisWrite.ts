// The fake's write gate (replica/redis/spec.md section 4), scripted like Rust's: Read-only refuses, Confirm writes
// answers with a `Pending` to show, Open runs, and deleting a key with more than 10,000 items asks for the
// database's name in every mode. A confirmation runs once, within two minutes, on the connection that asked.
import { fail, type Handlers } from '../types'
import type { Model } from '../model'
import type { RedisBytes, RedisWriteOp_Deserialize } from '../../../app/types/bindings'
import { MASTER } from './vault'
import { keyspace, type FakeKey } from './redisKeys'
import { deliver } from './redisPubSub'

type Op = RedisWriteOp_Deserialize
const BIG = 10_000
const id = (b: RedisBytes) => (b.kind === 'text' ? `t:${b.text}` : `b:${b.bytes.join(',')}`)
const show = (b: RedisBytes) => (b.kind === 'text' ? b.text : b.bytes.map(n => (n >= 0x21 && n < 0x7f && n !== 0x5c ? String.fromCharCode(n) : `\\x${n.toString(16).padStart(2, '0')}`)).join(''))
const text = (s: string): RedisBytes => ({ kind: 'text', text: s })
const RANK = { read_only: 2, confirm: 1, open: 0 } as const

function keysOf(m: Model, connId: string): FakeKey[] {
  const c = m.redisConns.get(connId)
  if (!c || c.closed) fail('not_found', 'That Redis connection is closed.')
  if (!m.redisKeys.length) m.redisKeys = keyspace()
  return m.redisKeys
}
const size = (k: FakeKey) => {
  const v = k.value
  return v.type === 'hash' ? v.fields.length : v.type === 'list' ? v.items.length : v.type === 'set' || v.type === 'zset' ? v.members.length : v.type === 'stream' ? v.entries.length : 0
}

/** The command as Rust's preview prints it: arguments with a space, a quote or nothing in them are quoted. */
function preview(parts: (string | RedisBytes)[]): string {
  return parts.map((p, i) => {
    const s = typeof p === 'string' ? p : show(p)
    const cut = s.length > 80 ? `${s.slice(0, 80)}…` : s
    return i > 0 && (cut === '' || cut.includes(' ') || cut.includes('"')) ? `"${cut.replaceAll('"', '\\"')}"` : cut
  }).join(' ')
}

/** What the op sends, and what it is about: the same words Rust uses. */
function describe(op: Op): { command: string, target: string, after: RedisBytes | null } {
  const inside = (key: RedisBytes, part: string) => `${show(key)} › ${part}`
  switch (op.op) {
    case 'set_string': return { command: preview(['SET', op.key, op.value, 'KEEPTTL']), target: show(op.key), after: op.value }
    case 'set_json': return { command: preview(['JSON.SET', op.key, '$', op.text]), target: show(op.key), after: text(op.text) }
    case 'hash_set': return { command: preview(['HSET', op.key, op.field, op.value]), target: inside(op.key, show(op.field)), after: op.value }
    case 'hash_remove': return { command: preview(['HDEL', op.key, op.field]), target: inside(op.key, show(op.field)), after: null }
    case 'list_push': return { command: preview([op.start ? 'LPUSH' : 'RPUSH', op.key, op.value]), target: inside(op.key, op.start ? 'start' : 'end'), after: op.value }
    case 'list_set': return { command: preview(['LSET', op.key, String(op.index), op.value]), target: inside(op.key, `#${op.index}`), after: op.value }
    case 'list_remove': return { command: `${preview(['LSET', op.key, String(op.index), '__dalan_removed__'])}\n${preview(['LREM', op.key, '1', '__dalan_removed__'])}`, target: inside(op.key, `#${op.index}`), after: null }
    case 'set_add': return { command: preview(['SADD', op.key, op.member]), target: inside(op.key, show(op.member)), after: null }
    case 'set_remove': return { command: preview(['SREM', op.key, op.member]), target: inside(op.key, show(op.member)), after: null }
    case 'zset_add': return { command: preview(['ZADD', op.key, op.score.trim(), op.member]), target: inside(op.key, show(op.member)), after: text(op.score.trim()) }
    case 'zset_remove': return { command: preview(['ZREM', op.key, op.member]), target: inside(op.key, show(op.member)), after: null }
    case 'stream_add': return { command: preview(['XADD', op.key, op.id ?? '*', ...op.fields.flatMap(f => [f.field, f.value])]), target: show(op.key), after: null }
    case 'stream_delete': return { command: preview(['XDEL', op.key, op.id]), target: inside(op.key, op.id), after: null }
    case 'expire': return { command: op.ttl_ms === null ? preview(['PERSIST', op.key]) : preview(['PEXPIRE', op.key, String(op.ttl_ms)]), target: show(op.key), after: null }
    case 'rename': return { command: preview(['RENAMENX', op.key, op.to]), target: show(op.key), after: op.to }
    case 'delete': return { command: preview(['UNLINK', op.key]), target: show(op.key), after: null }
    case 'create': return { command: preview([{ string: 'SET', json: 'JSON.SET', hash: 'HSET', list: 'RPUSH', set: 'SADD', zset: 'ZADD', stream: 'XADD' }[op.key_type] ?? 'SET', op.key, ...(op.field ? [op.field] : []), op.value]), target: show(op.key), after: op.value }
    case 'publish': return { command: preview(['PUBLISH', op.channel, op.message]), target: show(op.channel), after: op.message }
  }
}

/** What is there now, for "Now" in the confirmation. */
function before(keys: FakeKey[], op: Op): RedisBytes | null {
  const k = 'key' in op ? keys.find(x => id(x.key) === id(op.key)) : undefined
  const v = k?.value
  if (!v) return null
  if (op.op === 'set_string' && v.type === 'string') return v.value
  if (op.op === 'set_json' && v.type === 'json') return text(v.text)
  if ((op.op === 'hash_set' || op.op === 'hash_remove') && v.type === 'hash') { const f = v.fields.find(([n]) => n === show(op.field)); return f ? text(f[1]) : null }
  if ((op.op === 'zset_add' || op.op === 'zset_remove') && v.type === 'zset') { const z = v.members.find(([n]) => n === show(op.member)); return z ? text(String(z[1])) : null }
  if (op.op === 'list_set' || op.op === 'list_remove') return op.expect
  return null
}

const wrongType = (): never => fail('redis_refused', 'WRONGTYPE Operation against a key holding the wrong kind of value')

/** Changes the fake keyspace the way the command would. A collection that loses its last item is gone, like in Redis. */
function apply(keys: FakeKey[], op: Op, m: Model, send: (channel: number, message: unknown) => void): string | null {
  const find = (key: RedisBytes) => keys.find(x => id(x.key) === id(key))
  const drop = (key: RedisBytes) => { const i = keys.findIndex(x => id(x.key) === id(key)); if (i >= 0) keys.splice(i, 1) }
  const need = (key: RedisBytes) => find(key) ?? fail('not_found', 'That key is gone.')
  const emptied = (k: FakeKey) => { if (k.value.type !== 'string' && k.value.type !== 'json' && size(k) === 0) drop(k.key) }
  switch (op.op) {
    case 'publish': deliver(m, send, show(op.channel), show(op.message)); return null
    case 'create': {
      if (find(op.key)) fail('validation', 'A key with that name already exists. Nothing was changed.')
      const v = show(op.value)
      const f = op.field ? show(op.field) : ''
      keys.unshift({ key: op.key, ttl_ms: op.ttl_ms ?? null, value: op.key_type === 'hash' ? { type: 'hash', fields: [[f, v]] }
        : op.key_type === 'list' ? { type: 'list', items: [v] }
          : op.key_type === 'set' ? { type: 'set', members: [v] }
            : op.key_type === 'zset' ? { type: 'zset', members: [[v, Number(op.score ?? 0)]] }
              : op.key_type === 'stream' ? { type: 'stream', entries: [{ id: `${m.now}-0`, fields: [[f, v]] }], groups: [] }
                : op.key_type === 'json' ? { type: 'json', text: v } : { type: 'string', value: op.value } })
      return null
    }
    case 'delete': need(op.key); drop(op.key); return null
    case 'rename': {
      if (find(op.to)) fail('validation', 'A key with that name already exists. Nothing was changed.')
      need(op.key).key = op.to
      return null
    }
    case 'expire': need(op.key).ttl_ms = op.ttl_ms ?? null; return null
  }
  const k = need(op.key)
  const v = k.value
  switch (op.op) {
    case 'set_string': if (v.type !== 'string') wrongType(); else v.value = op.value; break
    case 'set_json': if (v.type !== 'json') wrongType(); else v.text = op.text; break
    case 'hash_set': {
      if (v.type !== 'hash') return wrongType()
      const f = v.fields.find(([n]) => n === show(op.field))
      if (f) f[1] = show(op.value)
      else v.fields.push([show(op.field), show(op.value)])
      break
    }
    case 'hash_remove': if (v.type !== 'hash') wrongType(); else v.fields = v.fields.filter(([n]) => n !== show(op.field)); break
    case 'list_push': if (v.type !== 'list') wrongType(); else if (op.start) v.items.unshift(show(op.value)); else v.items.push(show(op.value)); break
    case 'list_set':
    case 'list_remove': {
      if (v.type !== 'list') return wrongType()
      if (v.items[op.index] !== show(op.expect)) fail('validation', 'That item changed since you looked. Nothing was changed. Reload and try again.')
      if (op.op === 'list_set') v.items[op.index] = show(op.value)
      else v.items.splice(op.index, 1)
      break
    }
    case 'set_add': if (v.type !== 'set') wrongType(); else if (!v.members.includes(show(op.member))) v.members.push(show(op.member)); break
    case 'set_remove': if (v.type !== 'set') wrongType(); else v.members = v.members.filter(x => x !== show(op.member)); break
    case 'zset_add': {
      if (v.type !== 'zset') return wrongType()
      v.members = [...v.members.filter(([n]) => n !== show(op.member)), [show(op.member), Number(op.score)] as [string, number]].sort((a, b) => b[1] - a[1])
      break
    }
    case 'zset_remove': if (v.type !== 'zset') wrongType(); else v.members = v.members.filter(([n]) => n !== show(op.member)); break
    case 'stream_add': {
      if (v.type !== 'stream') return wrongType()
      const newId = op.id && op.id !== '*' ? op.id : `${m.now}-${v.entries.length}`
      v.entries.unshift({ id: newId, fields: op.fields.map(f => [show(f.field), show(f.value)] as [string, string]) })
      return newId
    }
    case 'stream_delete': if (v.type !== 'stream') wrongType(); else v.entries = v.entries.filter(e => e.id !== op.id); break
  }
  emptied(k)
  return null
}

function database(m: Model, connId: string) {
  const c = m.redisConns.get(connId)
  if (!c || c.closed) fail('not_found', 'That Redis connection is closed.')
  return m.redisDbs.find(d => d.id === c.db) ?? fail('not_found', 'That database is no longer saved.')
}
const readOnly = (name: string): never => fail('redis_read_only_mode', `${name} is read-only in Dalan. Nothing was sent.`)

type Keys = 'redisWritePrepare' | 'redisWriteCommit' | 'redisSetMode'
export const redisWrite: Pick<Handlers, Keys> = {
  redisWritePrepare: (a, { m, id: newId, tag, send }) => {
    const op = a.op as Op
    const db = database(m, a.connId)
    const keys = keysOf(m, a.connId)
    const k = op.op === 'delete' ? keys.find(x => id(x.key) === id(op.key)) : undefined
    if (op.op === 'delete' && !k) fail('not_found', 'That key is gone.')
    const dangerous = !!k && size(k) > BIG
    if (db.write_mode === 'read_only') {
      tag(`redis_refused:${op.op}`)
      readOnly(db.name)
    }
    if (db.write_mode === 'open' && !dangerous) {
      tag(`redis_write:${op.op}`)
      return { status: 'done', reply: apply(keys, op, m, send) }
    }
    const d = describe(op)
    const pendingId = newId('rpend')
    m.redisPending.set(pendingId, { connId: a.connId, op, typed: dangerous, at: Date.now() })
    return { status: 'pending', pending: { id: pendingId, level: dangerous ? 'typed' : 'confirm', command_preview: d.command, target: d.target, before: before(keys, op), after: d.after, count: k && k.value.type !== 'string' ? size(k) : null } }
  },
  redisWriteCommit: (a, { m, tag, send }) => {
    const db = database(m, a.connId)
    const p = m.redisPending.get(a.pendingId)
    // Used, unknown, or another tab's: all the same answer. A confirmation is used up when it is asked for.
    if (!p || p.connId !== a.connId) fail('redis_confirm_needed', 'That confirmation was already used or isn\'t for this tab. Nothing was sent.')
    m.redisPending.delete(a.pendingId)
    if (Date.now() - p.at >= 120_000) fail('redis_confirm_needed', 'That confirmation is more than two minutes old. Nothing was sent. Try the change again.')
    if (db.write_mode === 'read_only') readOnly(db.name)
    if (p.typed && a.typedName !== db.name) fail('redis_confirm_needed', 'The name doesn\'t match the database\'s name. Nothing was sent.')
    tag(`redis_write:${p.op.op}`)
    return { status: 'done', reply: apply(keysOf(m, a.connId), p.op, m, send) }
  },
  redisSetMode: (a, { m }) => {
    const db = m.redisDbs.find(d => d.id === a.dbId) ?? fail('not_found', 'That database is no longer saved.')
    const mode = a.mode as keyof typeof RANK
    if (db.environment === 'production' && RANK[mode] < RANK[db.write_mode]) {
      if (Date.now() < m.vault.cooldownUntil) fail('too_many_attempts', 'Too many wrong tries. Wait 30 seconds.')
      if (!a.masterPassword) fail('wrong_password', 'Type the master password to make a production database less protected.')
      if (a.masterPassword !== MASTER) {
        m.vault.wrongTries++
        if (m.vault.wrongTries >= 5) {
          m.vault.cooldownUntil = Date.now() + 30_000
          m.vault.wrongTries = 0
        }
        fail('wrong_password', 'That password didn\'t work.')
      }
      m.vault.wrongTries = 0
    }
    db.write_mode = mode
    return {
      id: db.id, name: db.name, environment: db.environment, route: db.route, host_id: db.host_id, host_label: m.hosts.find(h => h.id === db.host_id)?.label ?? null,
      address: db.address, port: db.port, kind: db.kind, sentinel_master: db.sentinel_master, username: db.username, has_password: db.password, tls: db.tls,
      tls_server_name: db.tls_server_name, write_mode: db.write_mode, default_db: db.default_db, key_separator: db.key_separator, last_opened_at: db.last_opened_at,
    }
  },
}
