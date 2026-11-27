// SQL tabs (replica/databases/spec.md D2): a connection that plays its events by itself, and one made-up schema
// (`public`: customers, orders, order_items, a view and a keyless table) served to every PostgreSQL database.
// Orders has 1,200 rows so paging, sorting and the estimate have something to do.
import { fail, type Handlers } from '../types'
import { stopCopy } from './copy'
import { stopRestore } from './restore'
import type { SqlCell_Serialize as Cell_Serialize, SqlColumn_Serialize, SqlTableInfo_Serialize } from '../../../app/types/bindings'

type Keys = 'dbOpen' | 'dbRespond' | 'dbClose' | 'sqlObjects' | 'sqlTableInfo' | 'sqlTablePage' | 'sqlCount' | 'sqlCell' | 'sqlStructure' | 'sqlStructurePrepare' | 'sqlStructureCommit' | 'sqlStructureDiscard' | 'sqlExplain' | 'sqlDiagram' | 'sqlSaveExport' | 'sqlActivity' | 'sqlSessionPrepare' | 'sqlSessionCommit' | 'sqlSessionDiscard' | 'sqlExportTable' | 'sqlTransferStop' | 'sqlImportPick' | 'sqlImportPreview' | 'sqlImportPrepare' | 'sqlImportCommit' | 'sqlImportDiscard' | 'sqlBackupCheck' | 'sqlBackupStart' | 'sqlRun' | 'sqlRunCommit' | 'sqlRunDiscard' | 'sqlTxn' | 'sqlGridPrepare' | 'sqlGridCommit' | 'sqlGridDiscard' | 'sqlCancel' | 'sqlEditorClose' | 'sqlCatalog' | 'sqlFormat' | 'sqlHistory' | 'sqlHistoryClear'

const col = (name: string, type_name: string, kind: SqlColumn_Serialize['kind'], pk: number | null = null, nullable = true): SqlColumn_Serialize =>
  ({ name, type_name, kind, nullable: pk === null && nullable, default: null, pk_position: pk })

const TABLES: Record<string, { cols: SqlColumn_Serialize[], rows: () => (string | null)[][], estimate: number | null, kind: 'table' | 'view' }> = {
  customers: {
    cols: [col('id', 'bigint', 'int', 1), col('name', 'text', 'text', null, false), col('meta', 'jsonb', 'json')],
    rows: () => Array.from({ length: 50 }, (_, i) => [String(i + 1), `customer ${i + 1}`, `{"vip": ${(i + 1) % 7 === 0}}`]),
    estimate: 50, kind: 'table',
  },
  orders: {
    cols: [col('id', 'bigint', 'int', 1), col('customer_id', 'bigint', 'int', null, false), col('status', 'order_status', 'enum', null, false), col('total', 'numeric(12,2)', 'decimal'), col('note', 'text', 'text')],
    rows: () => Array.from({ length: 1200 }, (_, i) => { const g = i + 1; return [String(g), String(1 + (g % 50)), ['open', 'paid', 'void'][g % 3]!, (g * 1.25).toFixed(2), g % 10 === 0 ? null : `note ${g}`] }),
    estimate: 1180, kind: 'table',
  },
  order_items: {
    cols: [col('order_id', 'bigint', 'int', 1), col('line_no', 'integer', 'int', 2), col('sku', 'text', 'text')],
    rows: () => Array.from({ length: 120 }, (_, i) => [String(Math.floor(i / 4) + 1), String((i % 4) + 1), `sku-${i}`]),
    estimate: 120, kind: 'table',
  },
  open_orders: { cols: [col('id', 'bigint', 'int'), col('status', 'order_status', 'enum')], rows: () => [['3', 'open'], ['6', 'open']], estimate: null, kind: 'view' },
  no_key: { cols: [col('a', 'integer', 'int'), col('b', 'text', 'text')], rows: () => [['1', 'a'], ['2', 'b']], estimate: 2, kind: 'table' },
}
const FKS = {
  orders: { references: [{ name: 'orders_customer_id_fkey', columns: ['customer_id'], schema: 'public', table: 'customers', target_columns: ['id'] }], referenced_by: [{ name: 'order_items_order_id_fkey', columns: ['order_id'], schema: 'public', table: 'order_items', target_columns: ['id'] }] },
  order_items: { references: [{ name: 'order_items_order_id_fkey', columns: ['order_id'], schema: 'public', table: 'orders', target_columns: ['id'] }], referenced_by: [] },
  customers: { references: [], referenced_by: [{ name: 'orders_customer_id_fkey', columns: ['customer_id'], schema: 'public', table: 'orders', target_columns: ['id'] }] },
} as Record<string, Pick<SqlTableInfo_Serialize, 'references' | 'referenced_by'>>

// Rows changed by a grid commit (D4): `table|pk|column` -> text, and deleted rows `table|pk`.
const patched = new Map<string, string | null>()
const deleted = new Set<string>()
const liveRows = (name: string) => {
  const t = TABLES[name]!
  return t.rows().filter(r => !deleted.has(`${name}|${r[0]}`)).map(r => r.map((v, i) => (patched.has(`${name}|${r[0]}|${t.cols[i]!.name}`) ? patched.get(`${name}|${r[0]}|${t.cols[i]!.name}`)! : v)))
}
const txns = new Map<string, { statements: number, since: number }>()
const held = new Map<string, { pieces: { text: string, start: number, end: number }[], editorId: string, level: string }>()
const gridHeld = new Map<string, { schema: string, table: string, rows: any[] }>()

type Cls = 'read' | 'write' | 'schema' | 'dangerous'
function classOf(t: string): Cls {
  const w = (/^\s*(\w+)/.exec(t)?.[1] ?? '').toLowerCase()
  if (['select', 'show', 'with', 'values', 'table', 'explain'].includes(w)) return 'read'
  if (w === 'drop' || w === 'truncate') return 'dangerous'
  if ((w === 'update' || w === 'delete') && !/\bwhere\b/i.test(t)) return 'dangerous'
  if (['create', 'alter', 'grant', 'revoke', 'vacuum', 'comment', 'analyze'].includes(w)) return 'schema'
  return 'write'
}
function decide(mode: string, c: Cls) {
  if (c === 'read') return 'run'
  if (mode === 'read_only') return 'refuse'
  if (c === 'write') return mode === 'open' ? 'run' : 'confirm'
  return c === 'schema' ? 'confirm' : 'typed'
}
const affectedOf = (t: string) => (classOf(t) === 'schema' ? 0 : /\bwhere\b/i.test(t) ? 1 : 50)

const structHeld = new Map<string, any[]>()
const tableKey = (t: string) => t

/** Statement text for one structure operation (the fake only shows it; Rust builds the real ones). */
function opSql(schema: string, tname: string, o: any) {
  const T = `"${schema}"."${tname}"`
  switch (o.op) {
    case 'add_column': return `alter table ${T} add column "${o.name}" ${o.type_name}${o.default ? ` default ${o.default}` : ''}${o.nullable ? '' : ' not null'}`
    case 'drop_column': return `alter table ${T} drop column "${o.name}"`
    case 'rename_column': return `alter table ${T} rename column "${o.from}" to "${o.to}"`
    case 'change_type': return `alter table ${T} alter column "${o.column}" type ${o.type_name} using "${o.column}"::${o.type_name}`
    case 'set_nullable': return `alter table ${T} alter column "${o.column}" ${o.nullable ? 'drop' : 'set'} not null`
    case 'set_default': return `alter table ${T} alter column "${o.column}" ${o.default ? `set default ${o.default}` : 'drop default'}`
    case 'create_index': return `create ${o.unique ? 'unique ' : ''}index "${o.name}" on ${T} (${o.columns.map((c: string) => `"${c}"`).join(', ')})`
    case 'drop_index': return `drop index "${schema}"."${o.name}"`
    case 'drop_constraint': return `alter table ${T} drop constraint "${o.name}"`
    default: return `alter table ${T} add constraint "${o.name}" foreign key (${o.columns.join(', ')}) references "${o.ref_schema}"."${o.ref_table}" (${o.ref_columns.join(', ')})`
  }
}
const extraIndexes = new Map<string, { name: string, definition: string, unique: boolean }[]>()

// ---- Activity (D6): a few made-up sessions; 18290 holds a lock that 18342 waits for.
const sessions = [
  { pid: 18342, user: 'api', application: 'api', state: 'active', age_s: 41, wait: 'Lock transactionid', query: 'update payments set state = \'settled\' where order_id = $1', blocked_by: [18290], mine: false },
  { pid: 18290, user: 'worker', application: 'worker', state: 'idle in transaction', age_s: 252, wait: 'Client', query: 'select * from payments where order_id = 5511 for update', blocked_by: [], mine: false },
  { pid: 18298, user: 'api', application: 'api', state: 'idle', age_s: 2, wait: '', query: 'select 1', blocked_by: [], mine: false },
  { pid: 18400, user: 'reporting', application: 'bi', state: 'idle', age_s: 600, wait: '', query: 'select 2', blocked_by: [], mine: false },
  { pid: 18377, user: 'app', application: 'Dalan', state: 'active', age_s: 0, wait: '', query: 'select pid from pg_stat_activity', blocked_by: [], mine: true },
]
let xacts = 1_000_000
const sessionHeld = new Map<string, { pid: number, end: boolean, typed: boolean }>()
const exportsDone: string[] = []
const importHeld = new Map<string, number>()

const LABEL: Record<string, string> = { postgres: 'PostgreSQL', mysql: 'MySQL', mariadb: 'MariaDB', mongodb: 'MongoDB' }
const VERSION: Record<string, string> = { postgres: 'PostgreSQL 16.4 on x86_64-pc-linux-gnu', mysql: 'MySQL 8.4.2', mariadb: 'MariaDB 11.4.2', mongodb: 'MongoDB 7.0.14' }

const table = (name: string) => TABLES[name] ?? fail('not_found', `There is no table public.${name} (it may have been dropped).`)
const cell = (c: SqlColumn_Serialize, v: string | null): Cell_Serialize => (v === null ? { kind: 'null', v: null, cut: false } : { kind: c.kind, v, cut: false })

/** Reads `col = value` pairs joined by `and`, and `status = 'open'`; anything else matches every row (the fake is not a SQL engine). */
function matcher(filter: string, cols: SqlColumn_Serialize[]): (row: (string | null)[]) => boolean {
  const parts = filter.split(/\s+and\s+/i).map(p => /^\(?"?(\w+)"?\s*(=|<=|>=|<|>)\s*'?([^']*)'?\)?$/.exec(p.trim())).filter(Boolean) as RegExpExecArray[]
  return (row) => parts.every((m) => {
    const i = cols.findIndex(c => c.name === m[1])
    if (i < 0) return true
    const v = row[i], x = m[3]!
    const num = Number.isFinite(Number(v)) && Number.isFinite(Number(x))
    const a = num ? Number(v) : v ?? '', b = num ? Number(x) : x
    return m[2] === '=' ? a === b : m[2] === '<=' ? a <= b : m[2] === '>=' ? a >= b : m[2] === '<' ? a < b : a > b
  })
}

// ---- The query editor (D3). Not a SQL engine: a script is split at `;`, and each statement is answered by what it
// mentions: `pg_sleep` waits until Stop, a table name returns that table's rows, `select 1` one row, anything that
// writes is refused like Rust does, and a statement naming `missing` fails with the server's words.
const stops = new Map<string, () => void>()
const history: string[] = []
const wait = (ms: number) => new Promise<void>(r => setTimeout(r, ms))

function split(text: string) {
  const out: { text: string, start: number, end: number }[] = []
  let start = 0
  for (const part of text.split(';')) {
    const lead = part.length - part.trimStart().length
    const t = part.trim()
    if (t) out.push({ text: t, start: start + lead, end: start + lead + t.length })
    start += part.length + 1
  }
  return out
}
async function runPieces(a: any, ch: number, send: (c: number, m: unknown) => void, pieces: { text: string, start: number, end: number }[]) {
    const started = Date.now()
    history.unshift(pieces.map(p => p.text).join(';\n'))
    let stopped = false
    stops.set(a.editorId, () => { stopped = true })
    for (const [i, p] of pieces.entries()) {
      if (stopped) break
      send(ch, { type: 'started', index: i, statement: p.text, start: p.start, end: p.end })
      if (/pg_sleep/i.test(p.text)) {
        const limit = (a.timeoutMs ?? 30000)
        for (let t = 0; t < limit && !stopped; t += 20) await wait(20)
        send(ch, { type: 'failed', index: i, kind: stopped ? 'cancelled' : 'timeout', message: 'canceling statement' })
        break
      }
      if (/\bmissing\b/i.test(p.text)) {
        send(ch, { type: 'failed', index: i, kind: 'error', message: 'relation "missing" does not exist' })
        break
      }
      if (classOf(p.text) !== 'read') {
        const t = txns.get(a.editorId)
        if (t) t.statements++
        // The one write shape the fake really applies: `update <table> set <col> = '<text>' where id = <n>`.
        const u = /^\s*update\s+(\w+)\s+set\s+(\w+)\s*=\s*'([^']*)'\s+where\s+id\s*=\s*(\d+)/i.exec(p.text)
        if (u && TABLES[u[1]!]) patched.set(`${u[1]}|${u[4]}|${u[2]}`, u[3]!)
        send(ch, { type: 'done', index: i, rows: 0, more: false, elapsed_ms: 3, affected: affectedOf(p.text) })
        continue
      }
      const tname = Object.keys(TABLES).find(n => new RegExp(`\\b${n}\\b`, 'i').test(p.text))
      let cols: SqlColumn_Serialize[]
      let rows: (string | null)[][]
      if (tname) {
        cols = TABLES[tname]!.cols
        rows = liveRows(tname)
      }
      else {
        cols = [col('one', 'integer', 'int'), col('nothing', 'text', 'text')]
        rows = [['1', null]]
      }
      send(ch, { type: 'columns', index: i, columns: cols.map(x => ({ name: x.name, type_name: x.type_name, kind: x.kind })) })
      const limit = a.limit || Infinity
      const shown = rows.slice(0, limit)
      for (let at = 0; at < shown.length; at += 500) send(ch, { type: 'rows', index: i, rows: shown.slice(at, at + 500).map(r => r.map((v, k) => cell(cols[k]!, v))) })
      send(ch, { type: 'done', index: i, rows: shown.length, more: rows.length > shown.length, elapsed_ms: 3, affected: null })
    }
    stops.delete(a.editorId)
    send(ch, { type: 'finished', elapsed_ms: Date.now() - started })

}

export const sql: Pick<Handlers, Keys> = {
  sqlActivity: (a) => {
    xacts += 500
    const tiles = { connections: 86, max_connections: 200, active: 7, waiting: 1, transactions_total: xacts, cache_hit_pct: 99.4, replicas: 2, replication_lag_s: 0.3, db_bytes: 41 * 2 ** 30 }
    const chain = sessions.filter(x => x.blocked_by.length)
    const link = (x: typeof sessions[number]) => ({ pid: x.pid, user: x.user, state: x.state, age_s: x.age_s, query: x.query })
    const chains = chain.length ? [{ root: link(sessions.find(x => x.pid === chain[0]!.blocked_by[0])!), waiting: chain.map(link) }] : []
    const tables: Record<string, { columns: string[], rows: string[][] }> = {
      locks: { columns: ['PID', 'User', 'Type', 'Relation', 'Mode', 'Granted'], rows: [['18290', 'worker', 'relation', 'payments', 'RowShareLock', 'true'], ['18342', 'api', 'transactionid', '', 'ShareLock', 'false']] },
      slow: { columns: ['PID', 'User', 'Running for', 'Query'], rows: [['18342', 'api', '00:00:41', 'update payments …']] },
      tables: { columns: ['Table', 'Rows', 'Size', 'Seq scans', 'Index scans', 'Dead rows', 'Last vacuum'], rows: [['public.orders', '1200', '96 kB', '3', '40', '0', 'never']] },
      roles: { columns: ['Role', 'Rights', 'Login', 'Connection limit', 'Sessions'], rows: [['app', '', 'login', '-1', '4']] },
      replication: { columns: ['Replica', 'Address', 'State', 'Sync', 'Replay lag'], rows: [['replica1', '10.0.0.7', 'streaming', 'async', '00:00:00.3']] },
    }
    return { tiles, sessions: a.view === 'sessions' ? sessions : [], chains: a.view === 'sessions' ? chains : [], table: a.view === 'sessions' ? null : tables[a.view]! }
  },
  sqlSessionPrepare: (a, { m }) => {
    const c = m.sqlConns.get(a.connId)
    const db = m.dbs.find(d => d.id === c?.db)
    if (!db) fail('not_found', 'That database connection is closed.')
    if (db.write_mode === 'read_only') fail('db_read_only_mode', `${db.name} is read-only in Dalan. Nothing was sent.`)
    const x = sessions.find(s => s.pid === a.pid && !s.mine)
    if (!x) fail('not_found', 'That session is gone, or it is one of Dalan\'s own.')
    const typed = db.environment === 'production'
    const id = `sess-${sessionHeld.size + 1}-${Date.now()}`
    sessionHeld.set(id, { pid: a.pid, end: a.action === 'end', typed })
    return { id, level: typed ? 'typed' : 'confirm', db_name: db.name, environment: db.environment, pid: a.pid, user: x.user, state: x.state, query: x.query, end: a.action === 'end' }
  },
  sqlSessionCommit: (a, { m }) => {
    const h = sessionHeld.get(a.pendingId)
    if (!h) fail('db_confirm_needed', 'That confirmation was already used or isn\'t for this tab. Nothing was sent.')
    const db = m.dbs.find(d => d.id === m.sqlConns.get(a.connId)!.db)!
    if (h.typed && a.typedName !== db.name) fail('db_confirm_needed', 'The name doesn\'t match the database\'s name. Nothing was sent.')
    sessionHeld.delete(a.pendingId)
    const i = sessions.findIndex(s => s.pid === h.pid)
    if (i < 0) return false
    if (h.end) {
      sessions.splice(i, 1)
      for (const s of sessions) s.blocked_by = s.blocked_by.filter(p => p !== h.pid)
    }
    else sessions[i]!.query = ''
    return true
  },
  sqlSessionDiscard: (a) => {
    sessionHeld.delete(a.pendingId)
    return null
  },
  sqlExportTable: (a, { channel, send, tag }) => {
    const ch = channel(a.progress)
    const rows = TABLES[a.table]?.rows().length ?? 0
    send(ch, { rows: Math.floor(rows / 2), bytes: 1024 })
    tag(`export:${a.table}.${a.format}`)
    exportsDone.push(a.table)
    return { path: `/tmp/${a.table}.${a.format}`, rows, bytes: rows * 20 }
  },
  sqlTransferStop: (a) => {
    stopRestore(a.transferId)
    stopCopy(a.transferId)
    return null
  },
  sqlImportPick: () => ({ id: 'file-1', name: 'orders-nov.csv' }),
  sqlImportPreview: (a) => {
    const cols = TABLES[a.table]?.cols ?? []
    const fileCols = ['id', 'who', 'name', 'note']
    return { file_columns: fileCols, sample: [['1', 'Ada', 'x', 'y']], total_rows: 3, table_columns: cols.map(c => ({ name: c.name, type_name: c.type_name, generated: false })), suggested: fileCols.map(f => cols.find(c => c.name === f)?.name ?? null) }
  },
  sqlImportPrepare: (a, { m }) => {
    const c = m.sqlConns.get(a.connId)
    const db = m.dbs.find(d => d.id === c!.db)!
    if (db.write_mode === 'read_only') fail('db_read_only_mode', `${db.name} is read-only in Dalan. Nothing was sent.`)
    const mapped = a.mapping.filter(Boolean)
    if (!mapped.length) fail('validation', 'Map at least one column.')
    const id = `imp-${importHeld.size + 1}-${Date.now()}`
    importHeld.set(id, 3)
    return { id, db_name: db.name, environment: db.environment, rows: 3, statement: `COPY "${a.schema}"."${a.table}" (${mapped.map((x: string) => `"${x}"`).join(', ')}) FROM STDIN WITH (FORMAT csv)` }
  },
  sqlImportCommit: (a) => {
    const n = importHeld.get(a.pendingId)
    if (n === undefined) fail('db_confirm_needed', 'That confirmation was already used or isn\'t for this tab. Nothing was sent.')
    importHeld.delete(a.pendingId)
    return n
  },
  sqlImportDiscard: (a) => {
    importHeld.delete(a.pendingId)
    return null
  },
  sqlBackupCheck: (a, { m }) => {
    const mongo = m.dbs.find(d => d.id === m.sqlConns.get(a.connId)?.db)?.engine === 'mongodb'
    return mongo ? { tool: 'mongodump', runs_on: 'db-primary', version: 'mongodump version: 100.9.4', problem: null } : { tool: 'pg_dump', runs_on: 'db-primary', version: 'pg_dump (PostgreSQL) 16.4', problem: null }
  },
  sqlBackupStart: (a, { channel, send, tag }) => {
    send(channel(a.progress), { rows: 0, bytes: 5 * 2 ** 20 })
    tag('backup:app.dump')
    return { path: '/tmp/app-2026-11-24.dump', rows: 0, bytes: 5 * 2 ** 20 }
  },
  sqlStructure: (a) => {
    const t = table(a.table)
    const pk = t.cols.filter(c => c.pk_position).map(c => c.name)
    const fks = FKS[a.table]?.references ?? []
    const constraints = [
      ...(pk.length ? [{ name: `${a.table}_pkey`, kind: 'p', definition: `PRIMARY KEY (${pk.join(', ')})`, columns: pk }] : []),
      ...fks.map(f => ({ name: f.name, kind: 'f', definition: `FOREIGN KEY (${f.columns.join(', ')}) REFERENCES ${f.schema}.${f.table}(${f.target_columns.join(', ')})`, columns: f.columns })),
    ]
    const indexes = [
      ...(pk.length ? [{ name: `${a.table}_pkey`, definition: `CREATE UNIQUE INDEX ${a.table}_pkey ON ${a.schema}.${a.table} USING btree (${pk.join(', ')})`, unique: true, primary: true, bytes: 16384, scans: 1200 }] : []),
      ...(extraIndexes.get(tableKey(a.table)) ?? []).map(i => ({ ...i, primary: false, bytes: 8192, scans: 0 })),
    ]
    const columns = t.cols.map(c => ({ name: c.name, type_name: c.type_name, nullable: c.nullable, default: c.default, generated: null }))
    const ddl = `create table "${a.schema}"."${a.table}" (\n${columns.map(c => `  "${c.name}" ${c.type_name}${c.nullable ? '' : ' not null'}`).join(',\n')}\n);\n`
    return { stats: { rows_estimate: t.estimate, table_bytes: 1_048_576, index_bytes: 65_536, last_vacuum: Date.now() - 2 * 3_600_000 }, columns, constraints, indexes, triggers: [], ddl }
  },
  sqlStructurePrepare: (a, { m }) => {
    const c = m.sqlConns.get(a.connId)
    const db = c && m.dbs.find(d => d.id === c.db)
    if (db?.write_mode === 'read_only') fail('db_read_only_mode', `${db.name} is read-only in Dalan. Nothing was sent.`)
    if (!a.ops.length) fail('validation', 'There are no changes to apply.')
    const statements = a.ops.map((o: any) => opSql(a.schema, a.table, o))
    const typed = a.ops.some((o: any) => ['drop_column', 'change_type', 'drop_index'].includes(o.op))
    const warnings = a.ops.flatMap((o: any) => (o.op === 'change_type' ? [`Changing the type of ${o.column} locks the table while it is rewritten.`] : o.op === 'drop_column' ? [`Dropping ${o.name} deletes everything stored in it.`] : []))
    const id = `struct-${structHeld.size + 1}-${Date.now()}`
    structHeld.set(id, a.ops.map((o: any) => ({ ...o, __table: a.table, __typed: typed })))
    return { id, level: typed ? 'typed' : 'confirm', db_name: db!.name, environment: db!.environment, statements, warnings }
  },
  sqlStructureCommit: (a, { m }) => {
    const ops = structHeld.get(a.pendingId)
    structHeld.delete(a.pendingId)
    if (!ops) fail('db_confirm_needed', 'That confirmation was already used or isn\'t for this tab. Nothing was sent.')
    const c = m.sqlConns.get(a.connId)!
    const db = m.dbs.find(d => d.id === c.db)!
    if (ops[0].__typed && a.typedName !== db.name) fail('db_confirm_needed', 'The name doesn\'t match the database\'s name. Nothing was sent.')
    const t = TABLES[ops[0].__table]!
    for (const o of ops) {
      if (o.op === 'add_column') { const old = t.rows; t.cols.push(col(o.name, o.type_name, 'text')); t.rows = () => old().map(r => [...r, o.default ?? null]) }
      if (o.op === 'drop_column') { const i = t.cols.findIndex(x => x.name === o.name); const old = t.rows; t.cols.splice(i, 1); t.rows = () => old().map(r => r.filter((_, k) => k !== i)) }
      if (o.op === 'change_type') { const x = t.cols.find(c => c.name === o.column); if (x) x.type_name = o.type_name }
      if (o.op === 'rename_column') { const x = t.cols.find(c => c.name === o.from); if (x) x.name = o.to }
      if (o.op === 'create_index') extraIndexes.set(tableKey(ops[0].__table), [...(extraIndexes.get(tableKey(ops[0].__table)) ?? []), { name: o.name, definition: `CREATE INDEX ${o.name} ON public.${ops[0].__table} USING btree (${o.columns.join(', ')})`, unique: o.unique }])
      if (o.op === 'drop_index') extraIndexes.set(tableKey(ops[0].__table), (extraIndexes.get(tableKey(ops[0].__table)) ?? []).filter(i => i.name !== o.name))
    }
    return null
  },
  sqlStructureDiscard: (a) => {
    structHeld.delete(a.pendingId)
    return null
  },
  sqlExplain: (a) => {
    const first = split(a.text)[0]?.text ?? ''
    if (a.analyze && classOf(first) !== 'read') fail('validation', 'Explain analyze runs the statement, so it only works for reads. Use plain Explain for a change.')
    const heavy = /orders/i.test(first)
    const node = (name: string, detail: string | null, est: number, got: number | null, self: number | null, pct: number | null, children: any[] = []) => ({ name, detail, est_rows: est, actual_rows: a.analyze ? got : null, self_ms: a.analyze ? self : null, pct: a.analyze ? pct : null, total_ms: a.analyze ? self : null, children })
    const root = node('Limit', null, 50, 50, 0.4, 0.5, [node('Hash Join', 'c.id = o.customer_id', 31200, 30877, 6.8, 8, [
      node('Seq Scan on orders o', heavy ? 'filter: status = \'paid\'' : null, 1200, 30877, 61, 75),
      node('Hash', null, 18412, 18412, 4.9, 6, [node('Seq Scan on customers c', null, 18412, 18412, 4.9, 6)]),
    ])])
    const hint = heavy && a.analyze ? { text: 'The scan on orders reads every row and takes 75% of the time. The planner expected 1200 rows and got 30877: statistics look stale. An index on orders (status) would let it skip the rest.', actions: [{ label: 'Open ANALYZE orders', sql: 'analyze "public"."orders"' }, { label: 'Open an index draft', sql: 'create index on "public"."orders" ("status")' }] } : null
    return { root, analyzed: a.analyze, planning_ms: 0.3, execution_ms: a.analyze ? 81.1 : null, buffers_read: a.analyze ? 2140 : null, hint }
  },
  sqlDiagram: (a) => {
    const start = a.tables.map((t: any) => t.table)
    const edges: any[] = []
    for (const [from, f] of Object.entries(FKS)) for (const r of f.references) edges.push({ name: r.name, from_schema: 'public', from_table: from, from_columns: r.columns, to_schema: 'public', to_table: r.table, to_columns: r.target_columns, on_delete: 'restrict', on_update: 'no action', indexed: true })
    const seen = new Set<string>(start)
    let frontier = [...start]
    for (let i = 0; i < a.steps; i++) {
      const next: string[] = []
      for (const e of edges) for (const [x, y] of [[e.from_table, e.to_table], [e.to_table, e.from_table]]) if (frontier.includes(x) && !seen.has(y)) { seen.add(y); next.push(y) }
      frontier = next
    }
    const names = [...seen].filter(n => TABLES[n])
    return {
      tables: names.map(n => ({ schema: 'public', name: n, columns: TABLES[n]!.cols.map(c => ({ name: c.name, type_name: c.type_name, pk: !!c.pk_position, fk: edges.some(e => e.from_table === n && e.from_columns.includes(c.name)) })) })),
      edges: edges.filter(e => names.includes(e.from_table) && names.includes(e.to_table)), total_tables: Object.keys(TABLES).length,
    }
  },
  sqlSaveExport: (a, { tag }) => {
    tag(`export:${a.suggestedName}:${a.data.length}`)
    return `/tmp/${a.suggestedName}`
  },
  dbOpen: (a, { m, id, channel, send }) => {
    const d = m.dbs.find(x => x.id === a.dbId)
    if (!d) fail('not_found', 'That database is no longer saved.')
    const connId = id('sconn')
    const events = channel(a.events)
    m.sqlConns.set(connId, { events, db: d.id, closed: false })
    d.last_opened_at = m.now
    const host = m.hosts.find(h => h.id === d.host_id)
    const steps = [
      ...(host ? [{ label: `Sign in to ${host.label}`, detail: null }] : []),
      { label: `Reach ${LABEL[d.engine as string] ?? 'PostgreSQL'} at ${d.address}:${d.port}`, detail: d.tls !== 'off' ? 'with TLS' : null },
      { label: d.username ? `Sign in as ${d.username}` : `Sign in to ${LABEL[d.engine as string] ?? 'PostgreSQL'}`, detail: null },
    ]
    queueMicrotask(() => {
      send(events, { type: 'plan', steps })
      steps.forEach((_s, index) => {
        send(events, { type: 'step', index, state: 'active', detail: null })
        send(events, { type: 'step', index, state: 'done', detail: null })
      })
      send(events, { type: 'ready', info: { engine: d.engine, version: VERSION[d.engine as string] ?? 'PostgreSQL 16.4 on x86_64-pc-linux-gnu', database: d.default_database ?? 'app', user: d.username ?? 'app', latency_ms: 4 } })
    })
    return { conn_id: connId }
  },
  dbRespond: (a, { m }) => {
    m.responses.push({ id: a.connId, prompt: a.promptId, answer: a.answer })
    return null
  },
  dbClose: (a, { m, tag }) => {
    const c = m.sqlConns.get(a.connId)
    if (c) c.closed = true
    tag(`db_close:${a.connId}`)
  },
  sqlObjects: (a, { m }) => {
    const c = m.sqlConns.get(a.connId)
    if (!c || c.closed) fail('not_found', 'That database connection is closed.')
    if (a.path.length === 0) return [{ name: 'public', kind: 'schema', estimate: null }, { name: 'billing', kind: 'schema', estimate: null }]
    if (a.path[0] === 'public') return Object.entries(TABLES).map(([name, t]) => ({ name, kind: t.kind, estimate: t.estimate }))
    return []
  },
  sqlTableInfo: (a) => {
    const t = table(a.table)
    return { columns: t.cols, references: FKS[a.table]?.references ?? [], referenced_by: FKS[a.table]?.referenced_by ?? [] }
  },
  sqlTablePage: (a) => {
    const r = a.request
    const t = table(r.table)
    if (r.filter.includes(';')) fail('validation', 'That isn\'t a filter I can use: write one condition, like status = \'open\' and total > 100')
    const shown = r.columns.length ? r.columns.map((n: string) => t.cols.findIndex(c => c.name === n)) : t.cols.map((_, i) => i)
    if (shown.some((i: number) => i < 0)) fail('validation', 'There is no such column in this table.')
    let rows = liveRows(r.table).filter(matcher(r.filter, t.cols))
    const pk = t.cols.findIndex(c => c.pk_position === 1)
    const sort = r.sort[0]
    const keyset = !sort && pk >= 0
    if (sort) {
      const i = t.cols.findIndex(c => c.name === sort.column)
      if (i < 0) fail('validation', `There is no column ${sort.column} to sort by.`)
      const num = t.cols[i]!.kind === 'int' || t.cols[i]!.kind === 'decimal'
      rows = [...rows].sort((x, y) => (num ? Number(x[i]) - Number(y[i]) : String(x[i]).localeCompare(String(y[i]))) * (sort.descending ? -1 : 1))
    }
    const size = Math.min(r.page_size, 1000)
    const start = keyset ? (r.cursor?.keys ? rows.findIndex(x => Number(x[pk]) > Number(r.cursor!.keys![0])) : 0) : (r.cursor?.offset ?? 0)
    const from = start < 0 ? rows.length : start
    const page = rows.slice(from, from + size)
    const more = from + size < rows.length
    const next = more ? (keyset ? { keys: [page[page.length - 1]![pk]!], offset: 0 } : { keys: null, offset: from + size }) : null
    const cols = shown.map((i: number) => t.cols[i]!)
    return { columns: cols, rows: page.map(row => shown.map((i: number) => cell(t.cols[i]!, row[i]!))), next, keyset, elapsed_ms: 7 }
  },
  sqlCount: (a) => {
    const t = table(a.table)
    return t.rows().filter(matcher(a.filter, t.cols)).length
  },
  sqlCell: (a) => {
    const t = table(a.table)
    const k = a.key[0]
    const pk = t.cols.findIndex(c => c.name === k?.column)
    const row = t.rows().find(x => x[pk] === k?.value)
    const ci = t.cols.findIndex(c => c.name === a.column)
    if (!row || ci < 0) fail('not_found', 'That row is gone.')
    return cell(t.cols[ci]!, row[ci]!)
  },
  sqlRun: async (a, { m, channel, send }) => {
    const c = m.sqlConns.get(a.connId)
    if (!c || c.closed) fail('not_found', 'That database connection is closed.')
    const ch = channel(a.events)
    const all = split(a.text)
    if (!all.length) fail('validation', 'There is nothing to run.')
    const pieces = a.mode === 'current' ? [all.find(p => (a.cursor ?? 0) >= p.start && (a.cursor ?? 0) <= p.end) ?? all.filter(p => p.end <= (a.cursor ?? 0)).at(-1) ?? all[0]!] : all
    const db = m.dbs.find(d => d.id === c.db)!
    const tc = pieces.findIndex(p => /^\s*(begin|start|commit|end|rollback|savepoint|release)\b/i.test(p.text))
    if (tc >= 0) {
      send(ch, { type: 'refused', index: tc, class: 'write', message: 'Transactions are started and ended with the Manual / Auto-commit button and the Commit and Roll back buttons, not with statements.' })
      send(ch, { type: 'finished', elapsed_ms: 0 })
      return { pending: null }
    }
    const classes = pieces.map(p => classOf(p.text))
    const order = ['run', 'confirm', 'typed', 'refuse']
    const decision = classes.map(k => decide(db.write_mode, k)).reduce((x, y) => (order.indexOf(y) > order.indexOf(x) ? y : x), 'run')
    if (decision === 'refuse') {
      const i = Math.max(0, classes.findIndex(k => k !== 'read'))
      send(ch, { type: 'refused', index: i, class: classes[i], message: `${db.name} is read-only in Dalan. Nothing was sent.` })
      send(ch, { type: 'finished', elapsed_ms: 0 })
      return { pending: null }
    }
    if (decision === 'confirm' || decision === 'typed') {
      const id = `pend-${held.size + 1}-${Date.now()}`
      held.set(id, { pieces, editorId: a.editorId, level: decision })
      return { pending: { id, level: decision === 'typed' ? 'typed' : 'confirm', db_name: db.name, environment: db.environment, statements: pieces.map((p, i) => ({ index: i, text: p.text, class: classes[i]!, note: /^(insert|update|delete)/i.test(p.text) ? `This will change ${affectedOf(p.text)} ${affectedOf(p.text) === 1 ? 'row' : 'rows'}.` : null })) } }
    }
    await runPieces(a, ch, send, pieces)
    return { pending: null }
  },
  sqlRunCommit: async (a, { m, channel, send }) => {
    const h = held.get(a.pendingId)
    held.delete(a.pendingId)
    if (!h) fail('db_confirm_needed', 'That confirmation was already used or isn\'t for this tab. Nothing was sent.')
    const c = m.sqlConns.get(a.connId)!
    const db = m.dbs.find(d => d.id === c.db)!
    if (h.level === 'typed' && a.typedName !== db.name) fail('db_confirm_needed', 'The name doesn\'t match the database\'s name. Nothing was sent.')
    await runPieces({ ...a, editorId: h.editorId }, channel(a.events), send, h.pieces)
    return null
  },
  sqlRunDiscard: (a) => {
    held.delete(a.pendingId)
    return null
  },
  sqlTxn: (a, { m }) => {
    const c = m.sqlConns.get(a.connId)
    const db = c && m.dbs.find(d => d.id === c.db)
    if (a.action === 'begin') {
      if (db?.write_mode === 'read_only') fail('db_read_only_mode', `${db.name} is read-only in Dalan. Nothing was sent.`)
      if (txns.has(a.editorId)) fail('sql_in_transaction', 'A transaction is already open in this editor.')
      txns.set(a.editorId, { statements: 0, since: Date.now() })
    }
    if (a.action === 'commit' || a.action === 'rollback') txns.delete(a.editorId)
    const t = txns.get(a.editorId)
    return { active: !!t, statements: t?.statements ?? 0, age_s: t ? Math.floor((Date.now() - t.since) / 1000) : 0, failed: false, locks: t && t.statements ? 2 : 0 }
  },
  sqlGridPrepare: (a, { m }) => {
    const c = m.sqlConns.get(a.connId)
    const db = c && m.dbs.find(d => d.id === c.db)
    if (db?.write_mode === 'read_only') fail('db_read_only_mode', `${db.name} is read-only in Dalan. Nothing was sent.`)
    const t = table(a.edits.table)
    if (!t.cols.some(x => x.pk_position)) fail('sql_no_key', 'This table has no key, so rows can\'t be matched safely. Use a query instead.')
    const q = (v: string | null) => (v === null ? 'NULL' : `'${v.replace(/'/g, '\'\'')}'`)
    const statements = a.edits.rows.map((r: any) => r.delete
      ? `delete from "${a.edits.schema}"."${a.edits.table}" where ${r.key.map((k: any) => `"${k.column}" = ${q(k.value)}`).join(' and ')}`
      : `update "${a.edits.schema}"."${a.edits.table}" set ${r.changes.map((x: any) => `"${x.column}" = ${q(x.new)}`).join(', ')} where ${r.key.map((k: any) => `"${k.column}" = ${q(k.value)}`).join(' and ')} and ${r.changes.map((x: any) => `"${x.column}" is not distinct from ${q(x.old)}`).join(' and ')}`)
    const id = `grid-${gridHeld.size + 1}-${Date.now()}`
    gridHeld.set(id, a.edits)
    return { id, db_name: db!.name, environment: db!.environment, statements }
  },
  sqlGridCommit: (a) => {
    const e = gridHeld.get(a.pendingId)
    gridHeld.delete(a.pendingId)
    if (!e) fail('db_confirm_needed', 'That confirmation was already used or isn\'t for this tab. Nothing was sent.')
    const t = table(e.table)
    const cur = liveRows(e.table)
    // The optimistic check first, so a stale row writes nothing at all.
    for (const [i, r] of e.rows.entries()) {
      const row = cur.find(x => x[0] === r.key[0].value)
      if (!row) fail('sql_conflict', `Row ${i + 1} was changed or removed by someone else since you loaded it. Nothing was written. Reload the table and try again.`)
      for (const ch of r.changes) if ((row[t.cols.findIndex(c => c.name === ch.column)] ?? null) !== ch.old) fail('sql_conflict', `Row ${i + 1} was changed or removed by someone else since you loaded it. Nothing was written. Reload the table and try again.`)
    }
    for (const r of e.rows) {
      const pk = r.key[0].value
      if (r.delete) deleted.add(`${e.table}|${pk}`)
      else for (const ch of r.changes) patched.set(`${e.table}|${pk}|${ch.column}`, ch.new)
    }
    return e.rows.length
  },
  sqlGridDiscard: (a) => {
    gridHeld.delete(a.pendingId)
    return null
  },
  sqlCancel: (a) => {
    stops.get(a.editorId)?.()
    return null
  },
  sqlEditorClose: () => null,
  sqlCatalog: () => ({ tables: Object.entries(TABLES).map(([name, t]) => ({ schema: 'public', name, columns: t.cols.map(c => c.name) })) }),
  sqlFormat: (a) => String(a.text).replace(/\s+/g, ' ').replace(/\b(select|from|where|order by|group by|limit)\b/gi, w => `\n${w.toUpperCase()}`).trim() + ';',
  sqlHistory: () => [...history],
  sqlHistoryClear: () => {
    history.length = 0
    return null
  },
}
