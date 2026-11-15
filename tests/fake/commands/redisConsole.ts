// The fake's console and Server view (replica/redis/spec.md, R4). A tiny interpreter over the fake keyspace, with
// the gate's decisions scripted like Rust's: reads run, writes follow the mode, KEYS and whole reads of big
// collections ask first in every mode, dangerous commands want the database's name, MONITOR and friends are refused.
import { fail, type Handlers } from '../types'
import type { Model } from '../model'
import { globToRegExp, keyspace, otherKeyspace, type FakeKey } from './redisKeys'

const READS = new Set(['PING', 'GET', 'TTL', 'PTTL', 'TYPE', 'EXISTS', 'STRLEN', 'HGET', 'HGETALL', 'HLEN', 'LRANGE', 'LLEN', 'SMEMBERS', 'SCARD', 'ZCARD', 'SCAN', 'INFO', 'DBSIZE', 'SELECT', 'ECHO', 'TIME'])
const REFUSED = new Set(['MONITOR', 'SUBSCRIBE', 'PSUBSCRIBE', 'SSUBSCRIBE', 'UNSUBSCRIBE', 'SYNC', 'PSYNC'])
const HOLDS = new Set(['BLPOP', 'BRPOP', 'WAIT', 'HELLO', 'RESET', 'QUIT', 'AUTH', 'WATCH'])
const SECRET = (a: string[]) => a[0] === 'AUTH' || (a[0] === 'ACL' && a[1]?.toUpperCase() === 'SETUSER') || (a[0] === 'CONFIG' && a[1]?.toUpperCase() === 'SET' && a.slice(2).some(x => /^(requirepass|masterauth|masteruser)$/i.test(x))) || ((a[0] === 'HELLO' || a[0] === 'MIGRATE') && a.slice(1).some(x => /^auth2?$/i.test(x)))

/** Splits a line like Rust's parser does (double and single quotes; `\n`, `\t`, `\"`, `\\`). */
export function split(line: string): string[] {
  const out: string[] = []
  const re = /"((?:[^"\\]|\\.)*)"|'((?:[^'\\]|\\.)*)'|(\S+)/g
  for (let m = re.exec(line); m; m = re.exec(line)) {
    if (m[1] !== undefined) out.push(m[1].replace(/\\(.)/g, (_x, c: string) => ({ n: '\n', t: '\t', r: '\r' }[c] ?? c)))
    else out.push(m[2] ?? m[3] ?? '')
  }
  if ((line.match(/(?<!\\)"/g)?.length ?? 0) % 2) fail('validation', 'A quote is never closed. Nothing was sent.')
  return out
}

function keysOf(m: Model, connId: string): FakeKey[] {
  const c = m.redisConns.get(connId)
  if (!c || c.closed) fail('not_found', 'That Redis connection is closed.')
  if (c.index !== 0) return otherKeyspace(c.index)
  if (!m.redisKeys.length) m.redisKeys = keyspace()
  return m.redisKeys
}
const name = (k: FakeKey) => (k.key.kind === 'text' ? k.key.text : '')
const q = (s: string) => `"${s.replaceAll('\\', '\\\\').replaceAll('"', '\\"')}"`
const list = (items: string[]) => (items.length ? items.map((x, i) => `${String(i + 1).padStart(String(items.length).length)}) ${q(x)}`) : ['(empty array)'])
const size = (k?: FakeKey) => {
  const v = k?.value
  return !v ? 0 : v.type === 'hash' ? v.fields.length : v.type === 'list' ? v.items.length : v.type === 'set' || v.type === 'zset' ? v.members.length : 0
}

type Class = 'read' | 'write' | 'heavy' | 'dangerous' | 'refused'
function classify(a: string[], keys: FakeKey[]): Class {
  const [cmd, sub] = [a[0]!, a[1]?.toUpperCase()]
  if (REFUSED.has(cmd) || (cmd === 'CLIENT' && sub === 'PAUSE')) return 'refused'
  if (['FLUSHDB', 'FLUSHALL', 'SHUTDOWN', 'DEBUG'].includes(cmd) || (cmd === 'CONFIG' && sub === 'SET') || (cmd === 'SLOWLOG' && sub === 'RESET')) return 'dangerous'
  if (cmd === 'KEYS') return 'heavy'
  const whole = cmd === 'SMEMBERS' || cmd === 'HGETALL' || (cmd === 'LRANGE' && a[2] === '0' && a[3] === '-1')
  if (whole && size(keys.find(k => name(k) === a[1])) > 10_000) return 'heavy'
  if (READS.has(cmd) || (cmd === 'CONFIG' && sub === 'GET') || (cmd === 'SLOWLOG' && sub === 'GET')) return 'read'
  return 'write'
}
const STRENGTH: Class[] = ['read', 'write', 'heavy', 'dangerous', 'refused']

/** Runs a command on the fake keyspace and prints the reply the way Rust does. */
function run(m: Model, connId: string, a: string[]): { lines: string[], error: boolean, selected: number | null } {
  const keys = keysOf(m, connId)
  const k = keys.find(x => name(x) === a[1])
  const ok = (lines: string[]) => ({ lines, error: false, selected: null })
  const err = (text: string) => ({ lines: [`(error) ${text}`], error: true, selected: null })
  const wrong = () => err('WRONGTYPE Operation against a key holding the wrong kind of value')
  switch (a[0]) {
    case 'PING': return ok([a[1] ? q(a[1]) : 'PONG'])
    case 'ECHO': return ok([q(a[1] ?? '')])
    case 'DBSIZE': return ok([`(integer) ${keys.length}`])
    case 'SELECT': {
      const c = m.redisConns.get(connId)!
      const index = Number(a[1])
      if (!Number.isInteger(index) || index < 0 || index > 15) return err('ERR DB index is out of range')
      c.index = index
      return { lines: ['OK'], error: false, selected: index }
    }
    case 'GET': return !k ? ok(['(nil)']) : k.value.type === 'string' && k.value.value.kind === 'text' ? ok([q(k.value.value.text)]) : wrong()
    case 'TTL': return ok([`(integer) ${!k ? -2 : k.ttl_ms === null ? -1 : Math.round(k.ttl_ms / 1000)}`])
    case 'EXISTS': return ok([`(integer) ${k ? 1 : 0}`])
    case 'TYPE': return ok([k ? k.value.type : 'none'])
    case 'HGETALL': return !k ? ok(['(empty array)']) : k.value.type === 'hash' ? ok(list(k.value.fields.flat())) : wrong()
    case 'SMEMBERS': return !k ? ok(['(empty array)']) : k.value.type === 'set' ? ok(list(k.value.members)) : wrong()
    case 'LRANGE': return !k ? ok(['(empty array)']) : k.value.type === 'list' ? ok(list(k.value.items)) : wrong()
    case 'KEYS': return ok(list(keys.map(name).filter(n => globToRegExp(a[1] ?? '*').test(n))))
    case 'SCAN': {
      const at = a.findIndex(x => x.toUpperCase() === 'MATCH')
      const found = keys.map(name).filter(n => (at < 0 ? true : globToRegExp(a[at + 1] ?? '*').test(n))).slice(0, 3)
      return ok(['1) "0"', ...found.map((n, i) => `${i === 0 ? '2) ' : '   '}${i + 1}) ${q(n)}`)])
    }
    case 'INFO': return ok(['# Server', 'redis_version:7.2.4', 'redis_mode:standalone'])
    case 'SET': {
      if (a.length < 3) return err('ERR wrong number of arguments for \'set\' command')
      if (k) k.value = { type: 'string', value: { kind: 'text', text: a[2]! } }
      else keys.unshift({ key: { kind: 'text', text: a[1]! }, ttl_ms: null, value: { type: 'string', value: { kind: 'text', text: a[2]! } } })
      return ok(['OK'])
    }
    case 'DEL': {
      const before = keys.length
      for (const n of a.slice(1)) { const i = keys.findIndex(x => name(x) === n); if (i >= 0) keys.splice(i, 1) }
      return ok([`(integer) ${before - keys.length}`])
    }
    case 'EXPIRE': if (k) k.ttl_ms = Number(a[2]) * 1000; return ok([`(integer) ${k ? 1 : 0}`])
    case 'FLUSHDB': keys.splice(0); return ok(['OK'])
    case 'SLOWLOG': if (a[1]?.toUpperCase() === 'RESET') m.redisSlowlog = []; return ok(['OK'])
    case 'BIGREPLY': return ok(Array.from({ length: Number(a[1] ?? 1500) }, (_, i) => `${String(i + 1).padStart(4)}) "row ${i + 1}"`))
    default: return err(`ERR unknown command '${a[0]}'`)
  }
}

function database(m: Model, connId: string) {
  const c = m.redisConns.get(connId)
  if (!c || c.closed) fail('not_found', 'That Redis connection is closed.')
  return m.redisDbs.find(d => d.id === c.db) ?? fail('not_found', 'That database is no longer saved.')
}

/** The server's commands for completion: a handful, with the syntax Redis 7 documents. */
const COMMANDS = [
  ['DEL', 'key [key ...]', 'Deletes one or more keys.', true], ['EXPIRE', 'key seconds [NX|XX|GT|LT]', 'Sets the expiration time of a key in seconds.', true],
  ['GET', 'key', 'Returns the string value of a key.', false], ['HGET', 'key field', 'Returns the value of a field in a hash.', false],
  ['HGETALL', 'key', 'Returns all fields and values in a hash.', false], ['HSET', 'key field value [field value ...]', 'Creates or modifies the value of a field in a hash.', true],
  ['HSETNX', 'key field value', 'Sets the value of a field in a hash only when the field doesn\'t exist.', true], ['KEYS', 'pattern', 'Returns all key names that match a pattern.', false],
  ['PING', '[message]', 'Returns the server\'s liveliness response.', false], ['SCAN', 'cursor [MATCH pattern] [COUNT count] [TYPE type]', 'Iterates over the key names in the database.', false],
  ['SET', 'key value [NX|XX] [GET] [EX seconds|PX milliseconds|KEEPTTL]', 'Sets the string value of a key.', true], ['TTL', 'key', 'Returns the expiration time in seconds of a key.', false],
] as const

type Keys = 'redisConsoleExec' | 'redisConsoleCommit' | 'redisConsoleHistory' | 'redisConsoleHistoryClear' | 'redisCommands' | 'redisInfo' | 'redisSlowlog'
export const redisConsole: Pick<Handlers, Keys> = {
  redisConsoleExec: (a, { m, id, tag }) => {
    const db = database(m, a.connId)
    const args = split(String(a.line))
    if (!args.length) fail('validation', 'Type a command.')
    args[0] = args[0]!.toUpperCase()
    if (!SECRET(args)) {
      const h = (m.redisHistory[db.id] ??= [])
      const line = String(a.line).trim()
      if (h[0] !== line) h.unshift(line)
    }
    if (HOLDS.has(args[0])) fail('redis_refused', 'Not sent. It would hold or change this tab\'s connection.')
    const reply = (r: { lines: string[], error: boolean, selected: number | null }) => ({ status: 'reply' as const, reply: { ...r, truncated: false, elapsed_ms: 0.4 } })
    const say = (text: string, error = false) => reply({ lines: [text], error, selected: null })
    const conn = m.redisConns.get(a.connId)!
    const queue = m.redisMulti.get(a.connId)
    if (args[0] === 'MULTI') { m.redisMulti.set(a.connId, []); return say('OK') }
    if (args[0] === 'DISCARD') { m.redisMulti.delete(a.connId); return say(queue ? 'OK' : '(error) ERR DISCARD without MULTI', !queue) }
    const keys = keysOf(m, a.connId)
    let cls: Class
    let lines: string[][]
    if (args[0] === 'EXEC') {
      if (!queue) return say('(error) ERR EXEC without MULTI', true)
      m.redisMulti.delete(a.connId)
      cls = queue.map(c => classify(c, keys)).reduce<Class>((x, y) => (STRENGTH.indexOf(y) > STRENGTH.indexOf(x) ? y : x), 'read')
      lines = queue
    }
    else {
      cls = classify(args, keys)
      lines = [args]
    }
    const refuse = (): never => {
      tag(`redis_refused:${args[0]}`)
      return cls === 'refused' ? fail('redis_refused', `Dalan doesn't send ${args[0]}. Nothing was sent.`) : fail('redis_read_only_mode', `${db.name} is read-only in Dalan. Nothing was sent.`)
    }
    if (cls === 'refused' || (db.write_mode === 'read_only' && (cls === 'write' || cls === 'dangerous'))) refuse()
    if (queue && args[0] !== 'EXEC') { queue.push(args); return say('QUEUED') }
    const ask = cls === 'heavy' || cls === 'dangerous' || (cls === 'write' && db.write_mode === 'confirm')
    if (!ask) {
      if (cls !== 'read') tag(`redis_write:${args[0]}`)
      const all = lines.map(l => run(m, a.connId, l))
      return reply(all.length === 1 ? all[0]! : { lines: all.flatMap((r, i) => r.lines.map((l, j) => `${j === 0 ? `${i + 1}) ` : '   '}${l}`)), error: false, selected: null })
    }
    void conn
    const pendingId = id('rpend')
    m.redisConsolePending.set(pendingId, { connId: a.connId, lines, typed: cls === 'dangerous', at: Date.now() })
    const safer = args[0] === 'KEYS' ? `SCAN 0 MATCH ${args[1] ?? '*'} COUNT 500` : args[0] === 'SMEMBERS' ? `SSCAN ${args[1]} 0 COUNT 500` : args[0] === 'HGETALL' ? `HSCAN ${args[1]} 0 COUNT 500` : null
    return { status: 'pending' as const, pending: { id: pendingId, level: cls === 'dangerous' ? 'typed' as const : 'confirm' as const, command_preview: lines.map(l => l.join(' ')).join('\n'), target: args[0], before: null, after: null, count: null }, heavy: cls === 'heavy', safer: cls === 'heavy' ? safer : null }
  },
  redisConsoleCommit: (a, { m, tag }) => {
    const db = database(m, a.connId)
    const p = m.redisConsolePending.get(a.pendingId)
    if (!p || p.connId !== a.connId) fail('redis_confirm_needed', 'That confirmation was already used or isn\'t for this tab. Nothing was sent.')
    m.redisConsolePending.delete(a.pendingId)
    if (Date.now() - p.at >= 120_000) fail('redis_confirm_needed', 'That confirmation is more than two minutes old. Nothing was sent. Try the change again.')
    if (p.typed && a.typedName !== db.name) fail('redis_confirm_needed', 'The name doesn\'t match the database\'s name. Nothing was sent.')
    tag(`redis_write:${p.lines[0]![0]}`)
    const all = p.lines.map(l => run(m, a.connId, l))
    const r = all.length === 1 ? all[0]! : { lines: all.flatMap((x, i) => x.lines.map((l, j) => `${j === 0 ? `${i + 1}) ` : '   '}${l}`)), error: false, selected: null }
    return { ...r, truncated: false, elapsed_ms: 1.2 }
  },
  redisConsoleHistory: (a, { m }) => [...(m.redisHistory[a.dbId] ?? [])],
  redisConsoleHistoryClear: (a, { m }) => {
    m.redisHistory[a.dbId] = []
    return null
  },
  redisCommands: () => COMMANDS.map(([name, syntax, summary, write]) => ({ name, syntax, summary, write })),
  redisInfo: (a, { m }) => {
    const keys = keysOf(m, a.connId)
    const expires = keys.filter(k => k.ttl_ms !== null)
    const db0 = m.redisConns.get(a.connId)!.index === 0 ? keys : (m.redisKeys.length ? m.redisKeys : keyspace())
    return {
      version: '7.2.4', mode: m.redisDbs.find(d => d.id === m.redisConns.get(a.connId)?.db)?.kind === 'cluster' ? 'cluster' : 'standalone', role: 'primary', uptime_secs: 41 * 86400 + 3600, memory_used: 1932735283, memory_max: 4294967296,
      clients: 212, blocked_clients: 0, ops_per_sec: 8420 + m.redisInfoCalls++, hits: 962, misses: 38,
      databases: db0.length ? [{ index: 0, keys: db0.length, expires: db0.filter(k => k.ttl_ms !== null).length || expires.length, avg_ttl_ms: 18_720_000 }, { index: 3, keys: 2, expires: 0, avg_ttl_ms: 0 }] : [{ index: 3, keys: 2, expires: 0, avg_ttl_ms: 0 }],
    }
  },
  redisSlowlog: (_a, { m }) => m.redisSlowlog,
}
