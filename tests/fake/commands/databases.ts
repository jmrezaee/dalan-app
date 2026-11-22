// The shared Databases commands (replica/databases/spec.md): every engine's saved connections, connection strings and
// saved queries. Redis rows come from `m.redisDbs`; the other engines are kept in `m.dbs`. A password is only a flag.
import { fail, type Handlers } from '../types'
import type { Model } from '../model'
import type { DatabaseRow_Serialize, DbUrlFields_Serialize, Engine } from '../../../app/types/bindings'
import { MASTER } from './vault'

type Keys = 'dbList' | 'dbSave' | 'dbDelete' | 'dbTest' | 'dbParseUrl' | 'dbEngines' | 'savedList' | 'savedSave' | 'savedDelete'

const PORT: Record<Engine, number> = { redis: 6379, postgres: 5432, mysql: 3306, mariadb: 3306, mongodb: 27017 }
const noOptions = { auth_source: null, replica_set: null, srv: false }
const PROTECTION = { read_only: 2, confirm: 1, open: 0 } as const

function redisRows(m: Model): DatabaseRow_Serialize[] {
  return m.redisDbs.map(d => ({
    id: d.id, engine: 'redis', name: d.name, environment: d.environment, route: d.route, host_id: d.host_id,
    host_label: m.hosts.find(h => h.id === d.host_id)?.label ?? null, address: d.address, port: d.port,
    default_database: String(d.default_db), username: d.username, has_password: d.password, tls: d.tls ? 'verify' : 'off',
    tls_server_name: d.tls_server_name, write_mode: d.write_mode, options: noOptions, last_opened_at: d.last_opened_at,
  }))
}

function check(m: Model, i: any) {
  if (i.engine === 'redis') fail('validation', 'Redis databases are saved from the Redis form.')
  if (!String(i.name ?? '').trim()) fail('validation', 'Give the database a name of up to 64 characters.')
  const address = String(i.address ?? '').trim()
  const entries = i.engine === 'mongodb' ? address.split(',').map(e => e.trim()).filter(Boolean) : [address]
  if (!address || entries.some(e => !e || /\s|\//.test(e)) || (i.engine !== 'mongodb' && address.includes(','))) {
    fail('validation', i.engine === 'mongodb' ? 'List the members as addresses with a comma between them, such as db1:27017,db2:27017.' : 'The address is a host name or an IP address, without spaces or a path.')
  }
  if (i.port === 0) fail('validation', 'The port must be between 1 and 65535.')
  if (i.engine !== 'mongodb' && i.port == null) fail('validation', 'Give the port.')
  if (i.route === 'host' && !i.host_id) fail('validation', 'Pick the host to go through.')
  if (i.route === 'host' && !m.hosts.some(h => h.id === i.host_id)) fail('validation', 'That host is gone. Pick another one.')
  if (i.engine !== 'mongodb' && (i.options?.replica_set || i.options?.auth_source || i.options?.srv)) fail('validation', 'Replica set, auth source and SRV are MongoDB settings.')
}

/** Reads a connection string the way Rust does (the password is never part of the answer). */
export function parseUrl(url: string): DbUrlFields_Serialize & { password: string | null } {
  const m = /^([a-z+]+):\/\/(.*)$/i.exec(url.trim())
  if (!m) fail('validation', 'That isn\'t a connection string. It starts with postgres://, mysql://, mariadb://, mongodb:// or redis://.')
  const scheme = m[1]!.toLowerCase()
  const engine = ({ postgres: 'postgres', postgresql: 'postgres', mysql: 'mysql', mariadb: 'mariadb', mongodb: 'mongodb', 'mongodb+srv': 'mongodb', redis: 'redis', rediss: 'redis' } as Record<string, Engine>)[scheme]
  if (!engine) fail('validation', 'This connection string\'s scheme isn\'t one Dalan opens.')
  const [beforeQuery = '', query = ''] = m[2]!.split('?')
  const slash = beforeQuery.indexOf('/')
  const authority = slash < 0 ? beforeQuery : beforeQuery.slice(0, slash)
  const path = slash < 0 ? '' : decodeURIComponent(beforeQuery.slice(slash + 1))
  const at = authority.lastIndexOf('@')
  const userinfo = at < 0 ? '' : authority.slice(0, at)
  const hosts = at < 0 ? authority : authority.slice(at + 1)
  if (!hosts) fail('validation', 'The connection string has no host.')
  const [user = '', ...pass] = userinfo.split(':')
  const q = new URLSearchParams(query)
  const sslmode = q.get('sslmode')
  const tls = scheme === 'mongodb+srv' || scheme === 'rediss' || q.get('tls') === 'true' || sslmode === 'verify-full' || sslmode === 'verify-ca' ? 'verify' : sslmode === 'require' ? 'require' : 'off'
  const mongo = engine === 'mongodb'
  const [host = '', port] = mongo ? [hosts, undefined] : hosts.split(':')
  return {
    engine, address: host, port: mongo ? null : port ? Number(port) : PORT[engine], username: user ? decodeURIComponent(user) : null,
    has_password: pass.length > 0 && pass.join(':') !== '', password: pass.length ? decodeURIComponent(pass.join(':')) : null,
    default_database: path || null, tls,
    options: { auth_source: q.get('authSource'), replica_set: q.get('replicaSet'), srv: scheme === 'mongodb+srv' },
  }
}

export const databases: Pick<Handlers, Keys> = {
  dbList: (_a, { m }) => [...redisRows(m), ...m.dbs].sort((a, b) => a.name.localeCompare(b.name)),
  dbSave: (a, { m, id }) => {
    const i = a.input
    check(m, i)
    const existing = i.id ? m.dbs.find(d => d.id === i.id) : undefined
    if (i.id && !existing) fail('not_found', 'That database is no longer saved.')
    if (existing?.environment === 'production' && (i.environment !== 'production' || PROTECTION[i.write_mode as keyof typeof PROTECTION] < PROTECTION[existing.write_mode])) {
      if (a.masterPassword !== MASTER) fail('wrong_password', 'Type the master password to make a production database less protected.')
    }
    const fromUrl = i.from_url ? parseUrl(i.from_url).password : null
    const hasPassword = i.clear_password ? false : !!(i.password || fromUrl) || !!existing?.has_password
    const row: DatabaseRow_Serialize = {
      id: existing?.id ?? id('db'), engine: i.engine, name: String(i.name).trim(), environment: i.environment, route: i.route,
      host_id: i.host_id || null, host_label: m.hosts.find(h => h.id === i.host_id)?.label ?? null, address: String(i.address).trim(),
      port: i.port ?? null, default_database: i.default_database || null, username: i.username || null, has_password: hasPassword,
      tls: i.tls, tls_server_name: i.tls_server_name || null, write_mode: i.write_mode, options: { ...noOptions, ...i.options }, last_opened_at: existing?.last_opened_at ?? null,
    }
    if (existing) Object.assign(existing, row)
    else m.dbs.push(row)
    return row
  },
  dbDelete: (a, { m }) => {
    const i = m.dbs.findIndex(d => d.id === a.id)
    const r = m.redisDbs.findIndex(d => d.id === a.id)
    if (i < 0 && r < 0) fail('not_found', 'That database is no longer saved.')
    if (i >= 0) m.dbs.splice(i, 1)
    else m.redisDbs.splice(r, 1)
    return null
  },
  dbTest: (a, { m }) => {
    check(m, a.input)
    return { engine: a.input.engine, latency_ms: 4, detail: `Reached ${a.input.address}.` }
  },
  dbParseUrl: (a) => {
    const { password: _password, ...fields } = parseUrl(a.url)
    return fields
  },
  dbEngines: () => ['postgres', 'mysql', 'mariadb', 'mongodb', 'redis'],
  savedList: (_a, { m }) => [...m.savedQueries].sort((a, b) => a.name.localeCompare(b.name)),
  savedSave: (a, { m, id }) => {
    const i = a.input
    if (!String(i.name ?? '').trim()) fail('validation', 'Give the query a name of up to 80 characters.')
    if (!String(i.body ?? '').trim()) fail('validation', 'There is nothing to save.')
    const existing = i.id ? m.savedQueries.find(q => q.id === i.id) : undefined
    const row = {
      id: existing?.id ?? id('sq'), database_id: i.database_id ?? null, database_name: m.dbs.find(d => d.id === i.database_id)?.name ?? null,
      engine: i.engine, kind: i.kind, folder: i.folder ?? null, name: String(i.name).trim(), description: i.description ?? null, body: i.body, updated_at: m.now,
    }
    if (existing) Object.assign(existing, row)
    else m.savedQueries.push(row)
    return row
  },
  savedDelete: (a, { m }) => {
    const i = m.savedQueries.findIndex(q => q.id === a.id)
    if (i < 0) fail('not_found', 'That saved query is gone.')
    m.savedQueries.splice(i, 1)
    return null
  },
}
