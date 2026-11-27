// Copy between databases (D13, S49, F32). The fake moves no rows: it plans from the made-up schema (every
// PostgreSQL database has the same tables, so a target that isn't new already has them) and keeps the gate's rules.
import { fail, type Handlers } from '../types'
import type { CopyObject_Deserialize, CopyPlan_Serialize, CopyReport_Serialize, CopyRequest_Deserialize, CopyTablePlan_Serialize } from '../../../app/types/bindings'

type Keys = 'copyPrepare' | 'copyCommit'

const held = new Map<string, { connId: string, typed: string, plan: CopyPlan_Serialize, slow: boolean }>()
const stops = new Set<string>()
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))
const EXISTING = ['customers', 'orders', 'order_items', 'open_orders', 'no_key']
const ROWS: Record<string, number> = { customers: 50, orders: 1200, order_items: 120, no_key: 2 }
const family = (e: string) => (e === 'mariadb' ? 'mysql' : e)

export function stopCopy(id: string) {
  stops.add(id)
}

export const copy: Pick<Handlers, Keys> = {
  copyPrepare: (a, { m, tag }) => {
    const db = (connId: string) => m.dbs.find(d => d.id === m.sqlConns.get(connId)?.db) ?? fail('not_found', 'That database connection is closed.')
    const src = db(a.connId)
    const dst = db(a.request.target_conn_id)
    const r = a.request as CopyRequest_Deserialize
    if (family(src.engine) !== family(dst.engine)) fail('validation', `Copying from ${src.engine} to ${dst.engine} arrives with the next update. This one copies between databases of the same engine.`)
    if (!r.objects.length) fail('validation', 'Choose at least one table to copy.')
    if (dst.write_mode === 'read_only') fail('db_read_only_mode', `${dst.name} is read-only in Dalan. Nothing was sent.`)
    if (dst.environment === 'production' && dst.write_mode !== 'open') fail('db_read_only_mode', `${dst.name} is a production database. To copy into it, switch it to Open first (that needs the master password).`)
    const newDb = (r.new_database ?? '').trim()
    const targetDb = newDb || dst.default_database || 'app'
    tag(`copy-request:${JSON.stringify({ objects: r.objects.map((o: CopyObject_Deserialize) => o.rename ? `${o.name}>${o.rename}` : o.name), mode: r.mode, content: r.content, newDb })}`)
    const tables: CopyTablePlan_Serialize[] = r.objects.map((o: CopyObject_Deserialize) => {
      const to = o.rename || o.name
      const exists = !newDb && EXISTING.includes(to)
      const action = r.mode === 'create' ? 'create' : r.mode === 'recreate' ? (exists ? 'drop and create' : 'create') : r.mode === 'empty' ? 'empty and load' : r.mode === 'append' ? 'add rows' : 'insert or replace'
      return { source: `${o.schema}.${o.name}`, target: `${o.schema}.${to}`, kind: o.kind, exists, action, rows_estimate: o.limit ?? ROWS[o.name] ?? null }
    })
    const there = tables.filter(t => t.exists && r.mode === 'create' && r.content !== 'data').map(t => t.target)
    const missing = tables.filter(t => !t.exists && ['empty', 'append', 'upsert'].includes(r.mode)).map(t => t.target)
    const typed = (tables.some(t => t.exists) && ['recreate', 'empty', 'upsert'].includes(r.mode)) || dst.environment === 'production'
    const plan: CopyPlan_Serialize = {
      pending_id: null,
      blocked: there.length ? `${there.length} already in ${targetDb}: ${there.join(', ')}. Choose “Drop and recreate”, “Empty then load”, “Add rows” or “Insert or replace”, or copy into a new database.` : missing.length ? `${missing.length} not in ${targetDb} yet: ${missing.join(', ')}. Choose “Create” for them.` : null,
      level: typed ? 'typed' : 'confirm',
      typed_name: dst.name,
      source_name: src.name,
      target_name: dst.name,
      environment: dst.environment,
      target_db: targetDb,
      new_database: !!newDb,
      tables,
      warnings: src.environment === 'production' && dst.environment !== 'production' ? [`Copying production data to ${dst.name}. Masking arrives later; check what the tables hold.`] : [],
      snapshot: src.engine !== 'mongodb',
    }
    if (!plan.blocked) {
      const id = `copy-${held.size + 1}-${Date.now()}`
      held.set(id, { connId: a.connId, typed: dst.name, plan, slow: m.restoreNext === 'slow' })
      plan.pending_id = id
    }
    return plan
  },
  copyCommit: async (a, { channel, send, tag }) => {
    const h = held.get(a.pendingId)
    if (!h || h.connId !== a.connId) fail('db_confirm_needed', 'That confirmation was already used or isn\'t for this tab. Nothing was sent.')
    if (h.plan.level === 'typed' && a.typedName !== h.typed) fail('db_confirm_needed', 'The name doesn\'t match the database\'s name. Nothing was sent.')
    held.delete(a.pendingId)
    const ch = channel(a.progress)
    const report: CopyReport_Serialize = { outcome: 'done', target_db: h.plan.target_db, tables: [], warnings: [], seconds: 3 }
    for (const [i, t] of h.plan.tables.entries()) {
      for (let k = 1; k <= (h.slow ? 30 : 2); k++) {
        if (stops.has(a.transferId)) {
          stops.delete(a.transferId)
          report.tables.push({ source: t.source, target: t.target, status: 'stopped', rows: 0, source_rows: null, target_rows: null, error: null, seconds: 1 })
          for (const x of h.plan.tables.slice(i + 1)) report.tables.push({ source: x.source, target: x.target, status: 'skipped', rows: 0, source_rows: null, target_rows: null, error: null, seconds: 0 })
          report.outcome = 'stopped'
          tag(`copy:stopped:${t.target}`)
          return report
        }
        send(ch, { table: t.source, index: i, tables: h.plan.tables.length, rows: ((t.rows_estimate ?? 0) * k) / (h.slow ? 30 : 2), estimate: t.rows_estimate, phase: 'rows' })
        await sleep(h.slow ? 80 : 5)
      }
      const n = t.rows_estimate ?? 0
      // `no_key` plays the table that fails, so the report and "run again" have something to show.
      if (t.source.endsWith('.no_key')) report.tables.push({ source: t.source, target: t.target, status: 'failed', rows: 0, source_rows: null, target_rows: null, error: 'duplicate key value violates unique constraint "no_key_pkey"', seconds: 1 })
      else report.tables.push({ source: t.source, target: t.target, status: 'done', rows: n, source_rows: n, target_rows: n, error: null, seconds: 1 })
    }
    if (report.tables.some(t => t.status === 'failed')) report.outcome = 'partial'
    tag(`copy:${h.plan.target_db}`)
    return report
  },
}
