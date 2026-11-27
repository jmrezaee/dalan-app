// Schema compare and sync (D16, E3). The fake has one fixed set of differences between any two different sides (and
// none between a side and itself), holds a sync like Rust does, and keeps snapshots in memory.
import { fail, type Handlers } from '../types'
import type { DiffItem_Serialize as DiffItem, Side, SnapshotRow } from '../../../app/types/bindings'

type Keys = 'schemaCompare' | 'schemaSyncScript' | 'schemaSyncPrepare' | 'schemaSyncCommit' | 'schemaSyncDiscard' | 'schemaScriptSave' | 'schemaSnapshotSave' | 'schemaSnapshots' | 'schemaSnapshotDelete'

const snapshots: SnapshotRow[] = []
const held = new Map<string, { connId: string, typed: string, dangerous: boolean, count: number }>()
const key = (s: Side) => (s.kind === 'snapshot' ? `snap:${s.id}` : `${s.conn_id}:${s.schema}`)

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
  schemaCompare: (a, { m, tag }) => {
    const label = (s: Side) => (s.kind === 'snapshot' ? `${snapshots.find(x => x.id === s.id)?.database_name ?? '?'} · snapshot` : `${m.dbs.find(d => d.id === m.sqlConns.get(s.conn_id)?.db)?.name ?? '?'} · ${s.schema}`)
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
