// Bulk export and import (D17, E6). The fake writes nothing: it reports what Rust would have written.
import { fail, type Handlers } from '../types'
import type { BulkExport_Deserialize as BulkExport, BulkItem_Deserialize as BulkItem } from '../../../app/types/bindings'

const held = new Map<string, { connId: string, items: BulkItem[] }>()
const mgHeld = new Map<string, { connId: string, typed: string | null, mode: string }>()

type Keys = 'bulkExport' | 'bulkImportPick' | 'bulkInfer' | 'bulkImportPrepare' | 'bulkImportCommit' | 'bulkImportDiscard' | 'mgImportPick' | 'mgImportPreview' | 'mgImportPrepare' | 'mgImportCommit' | 'mgImportDiscard'

export const bulkFake: Pick<Handlers, Keys> = {
  bulkExport: (a, { m, channel, send, tag }) => {
    const r = a.request as BulkExport
    if (!r.objects.length) fail('validation', 'Choose at least one table.')
    const db = m.dbs.find(d => d.id === m.sqlConns.get(a.connId)?.db)
    const mongo = db?.engine === 'mongodb'
    const ch = channel(a.progress)
    r.objects.forEach((o, i) => send(ch, { index: i, total: r.objects.length, table: `${o.schema}.${o.name}`, rows: 10 }))
    tag(`bulk-export:${r.zip ? 'zip' : 'folder'}:${mongo ? 'jsonl' : r.format}:${r.objects.map(o => `${o.schema}.${o.name}`).join(',')}:ddl=${r.ddl}:masks=${r.masks.map(x => `${x.column}=${x.kind}`).join(',')}`)
    return {
      name: `${db?.default_database ?? 'app'}-export-2026-11-27-1200${r.zip ? '.zip' : ''}`,
      files: r.objects.length,
      rows: r.objects.length * 10,
      warnings: mongo && r.masks.length ? ['Masking isn\'t applied to collections yet: their documents are exported as they are.'] : [],
    }
  },
  bulkImportPick: (a, { tag, id }) => {
    tag(`bulk-import-pick:${a.zip ? 'zip' : 'folder'}`)
    return {
      source: a.zip ? 'app-export.zip' : 'app-export',
      from: 'app-prod (2026-11-27T12:00:00Z)',
      skipped: ['notes.pdf'],
      files: [
        { id: id('file'), name: 'public.customers.csv', delimiter: ',', json: false, schema: 'public', table: 'customers', exists: true },
        { id: id('file'), name: 'public.orders.jsonl', delimiter: ',', json: true, schema: 'public', table: 'orders', exists: true },
        { id: id('file'), name: 'loyalty.csv', delimiter: ',', json: false, schema: 'public', table: 'loyalty', exists: false },
      ],
    }
  },
  bulkInfer: a => ({ ddl: `CREATE TABLE ${a.schema}.${a.table} (\n  id bigint,\n  points bigint,\n  since date\n)`, columns: [{ name: 'id', type_name: 'bigint' }, { name: 'points', type_name: 'bigint' }, { name: 'since', type_name: 'date' }], from_manifest: false }),
  bulkImportPrepare: (a, { m, id, tag }) => {
    const items = a.items as BulkItem[]
    for (const it of items) {
      if (it.create && !/^\s*create table/i.test(it.create)) fail('validation', `The DDL for ${it.table} must be one CREATE TABLE and nothing else.`)
    }
    const db = m.dbs.find(d => d.id === m.sqlConns.get(a.connId)?.db)
    const pending = id('bulk')
    held.set(pending, { connId: a.connId, items })
    tag(`bulk-import:${items.map(i => `${i.name}>${i.schema}.${i.table}${i.create ? '+create' : ''}`).join(',')}`)
    return { id: pending, db_name: db?.name ?? '', environment: db?.environment ?? 'none', files: items.length, creates: items.filter(i => i.create).length, rows: items.length * 25 }
  },
  bulkImportCommit: (a, { channel, send }) => {
    const h = held.get(a.pendingId)
    if (!h || h.connId !== a.connId) fail('db_confirm_needed', 'That confirmation is no longer valid. Review again.')
    held.delete(a.pendingId)
    const ch = channel(a.progress)
    return h.items.map((it, i) => {
      send(ch, { index: i, total: h.items.length, table: `${it.schema}.${it.table}`, rows: 0 })
      return { name: it.name, table: `${it.schema}.${it.table}`, rows: 25, created: !!it.create, error: null }
    })
  },
  bulkImportDiscard: (a) => { held.delete(a.pendingId) },
  mgImportPick: (_a, { id }) => ({ id: id('file'), name: 'items.jsonl' }),
  mgImportPreview: () => ({ format: 'json_lines', docs: 3, sample: ['{"sku": "a", "qty": 1}', '{"sku": "b", "qty": 2}', '{"_id": 7, "sku": "c"}'], fields: ['sku', 'qty', '_id'] }),
  mgImportPrepare: (a, { m, id, tag }) => {
    const db = m.dbs.find(d => d.id === m.sqlConns.get(a.connId)?.db)
    const replace = a.mode.mode === 'replace'
    const pending = id('mgimp')
    mgHeld.set(pending, { connId: a.connId, typed: replace ? db?.name ?? '' : null, mode: a.mode.mode })
    tag(`mongo-import:${a.db}.${a.coll}:${a.mode.mode}${a.mode.mode === 'upsert' ? `:${a.mode.field}` : ''}`)
    return { id: pending, level: replace ? 'typed' : 'confirm', typed_name: db?.name ?? '', environment: db?.environment ?? 'none', docs: 3 }
  },
  mgImportCommit: (a) => {
    const h = mgHeld.get(a.pendingId)
    if (!h || h.connId !== a.connId) fail('db_confirm_needed', 'That confirmation is no longer valid. Review again.')
    if (h.typed !== null && a.typedName !== h.typed) fail('db_confirm_needed', 'The name doesn\'t match the database\'s name. Nothing was sent.')
    mgHeld.delete(a.pendingId)
    if (h.mode === 'upsert') return { inserted: 0, upserted: 1, updated: 2, removed: 0, failed: [], failed_count: 0 }
    return { inserted: h.mode === 'insert' ? 2 : 3, upserted: 0, updated: 0, removed: h.mode === 'replace' ? 40 : 0, failed: h.mode === 'insert' ? ['E11000 duplicate key error dup key: { _id: 7 }'] : [], failed_count: h.mode === 'insert' ? 1 : 0 }
  },
  mgImportDiscard: (a) => { mgHeld.delete(a.pendingId) },
}
