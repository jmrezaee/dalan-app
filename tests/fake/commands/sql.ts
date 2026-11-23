// SQL tabs (replica/databases/spec.md D2): a connection that plays its events by itself, and one made-up schema
// (`public`: customers, orders, order_items, a view and a keyless table) served to every PostgreSQL database.
// Orders has 1,200 rows so paging, sorting and the estimate have something to do.
import { fail, type Handlers } from '../types'
import type { SqlCell_Serialize as Cell_Serialize, SqlColumn_Serialize, SqlTableInfo_Serialize } from '../../../app/types/bindings'

type Keys = 'dbOpen' | 'dbRespond' | 'dbClose' | 'sqlObjects' | 'sqlTableInfo' | 'sqlTablePage' | 'sqlCount' | 'sqlCell' | 'sqlRun' | 'sqlCancel' | 'sqlEditorClose' | 'sqlCatalog' | 'sqlFormat' | 'sqlHistory' | 'sqlHistoryClear'

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
const WRITES = /^(insert|update|delete|create|alter|drop|truncate|grant|revoke|vacuum|begin|commit|rollback|set|copy|call|do)\b/i

export const sql: Pick<Handlers, Keys> = {
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
      { label: `Reach PostgreSQL at ${d.address}:${d.port}`, detail: d.tls !== 'off' ? 'with TLS' : null },
      { label: d.username ? `Sign in as ${d.username}` : 'Sign in to PostgreSQL', detail: null },
    ]
    queueMicrotask(() => {
      send(events, { type: 'plan', steps })
      steps.forEach((_s, index) => {
        send(events, { type: 'step', index, state: 'active', detail: null })
        send(events, { type: 'step', index, state: 'done', detail: null })
      })
      send(events, { type: 'ready', info: { engine: 'postgres', version: 'PostgreSQL 16.4 on x86_64-pc-linux-gnu', database: d.default_database ?? 'app', user: d.username ?? 'app', latency_ms: 4 } })
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
    let rows = t.rows().filter(matcher(r.filter, t.cols))
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
    const started = Date.now()
    const refused = pieces.findIndex(p => WRITES.test(p.text))
    if (refused >= 0) {
      send(ch, { type: 'refused', index: refused, class: 'write', message: 'This changes data or structure. The editor only runs reads for now; changes arrive with the confirmation dialogs in an update.' })
      send(ch, { type: 'finished', elapsed_ms: 0 })
      return null
    }
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
      const tname = Object.keys(TABLES).find(n => new RegExp(`\\b${n}\\b`, 'i').test(p.text))
      let cols: SqlColumn_Serialize[]
      let rows: (string | null)[][]
      if (tname) {
        cols = TABLES[tname]!.cols
        rows = TABLES[tname]!.rows()
      }
      else {
        cols = [col('one', 'integer', 'int'), col('nothing', 'text', 'text')]
        rows = [['1', null]]
      }
      send(ch, { type: 'columns', index: i, columns: cols.map(x => ({ name: x.name, type_name: x.type_name, kind: x.kind })) })
      const limit = a.limit || Infinity
      const shown = rows.slice(0, limit)
      for (let at = 0; at < shown.length; at += 500) send(ch, { type: 'rows', index: i, rows: shown.slice(at, at + 500).map(r => r.map((v, k) => cell(cols[k]!, v))) })
      send(ch, { type: 'done', index: i, rows: shown.length, more: rows.length > shown.length, elapsed_ms: 3 })
    }
    stops.delete(a.editorId)
    send(ch, { type: 'finished', elapsed_ms: Date.now() - started })
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
