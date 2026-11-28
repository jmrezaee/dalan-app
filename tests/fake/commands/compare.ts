// Schema compare and sync (D16, E3). The fake has one fixed set of differences between any two different sides (and
// none between a side and itself), holds a sync like Rust does, and keeps snapshots in memory.
import { fail, type Handlers } from '../types'
import type { DiffItem_Serialize as DiffItem, Side_Deserialize as Side, SnapshotRow } from '../../../app/types/bindings'

type Keys = 'dataColumns' | 'dataCompare' | 'dataMatchPrepare' | 'dataMatchCommit' | 'dataMatchDiscard' | 'dataMatchScript' | 'schemaCompare' | 'schemaSyncScript' | 'schemaSyncPrepare' | 'schemaSyncCommit' | 'schemaSyncDiscard' | 'schemaScriptSave' | 'schemaSnapshotSave' | 'schemaSnapshots' | 'schemaSnapshotDelete'

const snapshots: SnapshotRow[] = []
const held = new Map<string, { connId: string, typed: string, dangerous: boolean, count: number }>()
const key = (s: Side) => (s.kind === 'snapshot' ? `snap:${s.id}` : s.kind === 'dump' ? `dump:${s.dump_id}` : `${s.conn_id}:${s.schema}`)
const matches = new Map<string, { connId: string, typed: string, production: boolean }>()

const ITEMS: DiffItem[] = [
  { id: 'table:orders', kind: 'table', table: null, name: 'orders', change: 'added', source_ddl: 'CREATE TABLE orders (\n  id bigint NOT NULL,\n  customer_id bigint NOT NULL\n)', target_ddl: null, detail: ['2 columns'], dangerous: false },
  { id: 'column:customers.id', kind: 'column', table: 'customers', name: 'id', change: 'changed', source_ddl: 'bigint NOT NULL', target_ddl: 'integer NOT NULL', detail: ['type integer → bigint'], dangerous: true },
  { id: 'column:customers.tier', kind: 'column', table: 'customers', name: 'tier', change: 'added', source_ddl: 'text DEFAULT \'basic\'::text', target_ddl: null, detail: [], dangerous: false },
  { id: 'index:customers.customers_tier_idx', kind: 'index', table: 'customers', name: 'customers_tier_idx', change: 'added', source_ddl: 'CREATE INDEX customers_tier_idx ON customers USING btree (tier)', target_ddl: null, detail: [], dangerous: false },
  { id: 'column:customers.legacy', kind: 'column', table: 'customers', name: 'legacy', change: 'removed', source_ddl: null, target_ddl: 'text', detail: [], dangerous: true },
  { id: 'view:gone', kind: 'view', table: null, name: 'gone', change: 'removed', source_ddl: null, target_ddl: 'CREATE VIEW gone AS\nSELECT 2 AS y', detail: [], dangerous: true },
]
const SQL: Record<string, string> = {
  'table:orders': 'CREATE TABLE orders (\n  id bigint NOT NULL,\n  customer_id bigint NOT NULL\n)',
  'column:customers.id': 'ALTER TABLE customers ALTER COLUMN id TYPE bigint USING id::bigint',
  'column:customers.tier': 'ALTER TABLE customers ADD COLUMN tier text DEFAULT \'basic\'::text',
  'index:customers.customers_tier_idx': 'CREATE INDEX customers_tier_idx ON customers USING btree (tier)',
  'column:customers.legacy': 'ALTER TABLE customers DROP COLUMN legacy',
  'view:gone': 'DROP VIEW gone',
}

function script(chosen: string[]) {
  const picked = ITEMS.filter(i => chosen.includes(i.id))
  if (!picked.length) fail('validation', 'Nothing to do: the chosen differences are gone (the schemas changed since the compare). Compare again.')
  const safe = picked.filter(i => i.change !== 'removed').map(i => `${SQL[i.id]};`)
  const drops = picked.filter(i => i.change === 'removed').map(i => `${SQL[i.id]};`)
  return { text: ['-- Make the target match the source', 'SET search_path TO public;', ...safe, ...(drops.length ? ['-- Drops: these remove objects and the data in them.', ...drops] : [])].join('\n\n'), picked }
}

export const compareFake: Pick<Handlers, Keys> = {
  dataColumns: (a) => {
    const noKey = a.source.table === 'no_key'
    return { columns: ['id', 'name', 'email', 'blob'].map(n => ({ name: n, kind: n === 'blob' ? 'binary' : n === 'id' ? 'num' : 'text', in_target: n !== 'email' || a.target.table === a.source.table })), primary_key: noKey ? [] : ['id'] }
  },
  dataCompare: (a, { channel, send, tag }) => {
    if (!a.key.length) fail('validation', `${a.source.schema}.${a.source.table} has no primary key: choose the columns that identify a row.`)
    send(channel(a.progress), { rows: 25000, ranges_done: 2, ranges: 3 })
    tag(`data-compare:${a.source.table}>${a.target.table}:${a.key.join(',')}:${a.quick}`)
    return {
      only_source: 2, only_target: 1, different: 3, identical: 24994,
      samples: [
        { kind: 'only_source', key: '7', changes: [] },
        { kind: 'only_source', key: '8', changes: [] },
        { kind: 'only_target', key: '90', changes: [] },
        { kind: 'different', key: '12', changes: [{ column: 'name', source: 'Ada', target: 'Ada L.' }] },
        { kind: 'different', key: '13', changes: [{ column: 'email', source: 'a@x.test', target: null }] },
        { kind: 'different', key: '14', changes: [{ column: 'name', source: 'Bo', target: 'Bob' }] },
      ],
      columns: ['name', 'email'], key: a.key, skipped: ['blob (binary)'], ranges: a.quick ? 3 : 0, ranges_same: a.quick ? 1 : 0, warnings: [], seconds: 1.2,
    }
  },
  dataMatchPrepare: (a, { m, id }) => {
    const db = m.dbs.find(d => d.id === m.sqlConns.get(a.target.conn_id)?.db)
    const production = db?.environment === 'production'
    const pending = id('match')
    matches.set(pending, { connId: a.target.conn_id, typed: db?.name ?? '', production })
    return { pending_id: pending, level: production ? 'typed' : 'confirm', typed_name: db?.name ?? '', environment: db?.environment ?? 'none' }
  },
  dataMatchCommit: (a, { tag }) => {
    const h = matches.get(a.pendingId)
    if (!h || h.connId !== a.connId) fail('db_confirm_needed', 'That confirmation is no longer valid. Review again.')
    if (h.production && a.typedName !== h.typed) fail('db_confirm_needed', 'The name doesn\'t match the database\'s name. Nothing was sent.')
    matches.delete(a.pendingId)
    tag('data-match-commit')
    return { inserted: 2, updated: 3, deleted: 1, seconds: 0.4 }
  },
  dataMatchDiscard: (a) => { matches.delete(a.pendingId) },
  dataMatchScript: (_a, { tag }) => {
    tag('data-match-script')
    return 'match-customers-2026-11-27-1200.sql'
  },
  schemaCompare: (a, { m, tag }) => {
    const label = (s: Side) => (s.kind === 'snapshot' ? `${snapshots.find(x => x.id === s.id)?.database_name ?? '?'} · snapshot` : s.kind === 'dump' ? `shop-2026-11-20.dump · ${s.schema || 'public'}` : `${m.dbs.find(d => d.id === m.sqlConns.get(s.conn_id)?.db)?.name ?? '?'} · ${s.schema}`)
    if (a.source.kind === 'dump' || a.target.kind === 'dump') {
      const d = (a.source.kind === 'dump' ? a.source : a.target) as { conn_id: string }
      if (m.dbs.find(x => x.id === m.sqlConns.get(d.conn_id)?.db)?.environment === 'production') fail('validation', 'Reading a dump needs a scratch database, and it is production. Pick a tab of another server.')
    }
    tag(`schema-compare:${key(a.source)}>${key(a.target)}`)
    const same = key(a.source) === key(a.target)
    return {
      family: 'pg',
      source_label: label(a.source),
      target_label: label(a.target),
      items: same ? [] : ITEMS,
      tables: same ? [] : [{ name: 'customers', source: 'CREATE TABLE customers (\n  id bigint NOT NULL,\n  tier text\n)', target: 'CREATE TABLE customers (\n  id integer NOT NULL,\n  legacy text\n)' }, { name: 'orders', source: SQL['table:orders']!, target: null }],
      can_sync: a.target.kind === 'database',
    }
  },
  schemaSyncScript: a => script(a.chosen).text,
  schemaSyncPrepare: (a, { m, id, tag }) => {
    if (a.target.kind !== 'database') fail('validation', 'A snapshot can\'t be changed: pick a database as the target.')
    const { text, picked } = script(a.chosen)
    const db = m.dbs.find(d => d.id === m.sqlConns.get((a.target as { conn_id: string }).conn_id)?.db)
    const dangerous = picked.some(i => i.dangerous)
    const pending = id('sync')
    held.set(pending, { connId: a.target.conn_id, typed: db?.name ?? '', dangerous, count: picked.length })
    tag(`schema-sync:${[...a.chosen].sort().join(',')}`)
    return { pending_id: pending, level: dangerous ? 'typed' : 'confirm', typed_name: db?.name ?? '', environment: db?.environment ?? 'none', script: text, statements: picked.length, dangerous: picked.filter(i => i.dangerous).length }
  },
  schemaSyncCommit: (a, { tag }) => {
    const h = held.get(a.pendingId)
    if (!h || h.connId !== a.connId) fail('db_confirm_needed', 'That confirmation is no longer valid. Review again.')
    if (h.dangerous && a.typedName !== h.typed) fail('db_confirm_needed', 'The name doesn\'t match the database\'s name. Nothing was sent.')
    held.delete(a.pendingId)
    tag('schema-sync-commit')
    return { ran: h.count, total: h.count, failed: null, rolled_back: false }
  },
  schemaSyncDiscard: (a) => { held.delete(a.pendingId) },
  schemaScriptSave: (a, { tag }) => {
    script(a.chosen)
    tag('schema-script-save')
    return 'migrate-public-2026-11-27-1200.sql'
  },
  schemaSnapshotSave: (a, { m, id }) => {
    const db = m.dbs.find(d => d.id === m.sqlConns.get(a.connId)?.db)
    if (!db) fail('not_found', 'That database connection is closed.')
    const row: SnapshotRow = { id: id('snap'), database_id: db.id, database_name: db.name, schema: a.schema, family: db.engine === 'postgres' ? 'pg' : 'my', captured_at: Date.now(), tables: 4 }
    snapshots.unshift(row)
    return row
  },
  schemaSnapshots: () => snapshots,
  schemaSnapshotDelete: (a) => {
    const i = snapshots.findIndex(s => s.id === a.id)
    if (i >= 0) snapshots.splice(i, 1)
    return null
  },
}
