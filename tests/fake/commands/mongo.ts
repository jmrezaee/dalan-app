// MongoDB tabs (replica/databases/spec.md D8): two databases, `shop` (orders with 600 documents, customers with 30, an
// `open_orders` view) and `admin`. Filters are exact matches on top-level fields; every other query shape is the
// real backend's job. Edits change the fake's documents so a test can see them come back.
import { fail, type Handlers } from '../types'

type Keys = 'mgTree' | 'mgFind' | 'mgCount' | 'mgDocGet' | 'mgDocPrepare' | 'mgDocCommit' | 'mgDocDiscard'
type Doc = Record<string, any>

const store: Record<string, Doc[]> = {}
function docs(db: string, coll: string): Doc[] {
  const key = `${db}.${coll}`
  if (!store[key]) {
    store[key] = db === 'shop' && coll === 'orders'
      ? Array.from({ length: 600 }, (_, i) => ({ _id: i + 1, status: ['open', 'paid', 'void'][(i + 1) % 3], qty: (i % 9) + 1, note: `order ${i + 1}`, placed: { $date: '2025-03-01T10:00:00Z' } }))
      : db === 'shop' && coll === 'customers'
        ? Array.from({ length: 30 }, (_, i) => ({ _id: i + 1, name: `customer ${i + 1}`, vip: (i + 1) % 7 === 0 }))
        : []
  }
  return store[key]!
}
// `_id` and `qty` are Int32 in the fake: canonical shows them wrapped, relaxed plain.
const canonical = (d: Doc) => JSON.stringify({ ...d, _id: { $numberInt: String(d._id) }, ...(d.qty !== undefined ? { qty: { $numberInt: String(d.qty) } } : {}) }, null, 2)
const unwrap = (v: string): Doc => JSON.parse(v, (_k, x) => (x && typeof x === 'object' && '$numberInt' in x ? Number(x.$numberInt) : x))
const toDoc = (d: Doc) => ({ id: JSON.stringify({ $numberInt: String(d._id) }), relaxed: JSON.stringify(d, null, 2), cut: false })
const parse = (t: string, what: string): Doc => {
  if (!t.trim()) return {}
  try { return unwrap(t) }
  catch (e) { return fail('validation', `${what} isn't valid JSON: ${(e as Error).message}`) }
}
const matches = (d: Doc, f: Doc) => Object.entries(f).every(([k, v]) => JSON.stringify(d[k]) === JSON.stringify(v))
const held = new Map<string, { db: string, coll: string, op: Doc }>()

export const mongo: Pick<Handlers, Keys> = {
  mgTree: (a, { m }) => {
    const c = m.sqlConns.get(a.connId)
    if (!c || c.closed) fail('not_found', 'That database connection is closed.')
    if (a.path.length === 0) return [{ name: 'admin', kind: 'database' }, { name: 'shop', kind: 'database' }]
    if (a.path[0] === 'shop') return [{ name: 'customers', kind: 'collection' }, { name: 'open_orders', kind: 'view' }, { name: 'orders', kind: 'collection' }]
    return []
  },
  mgFind: (a) => {
    const r = a.request
    const filter = parse(r.filter, 'The filter')
    parse(r.project, 'The projection')
    const all = docs(r.db, r.coll).filter(d => matches(d, filter))
    const sort = parse(r.sort, 'The sort')
    const [sk, dir] = Object.entries(sort)[0] ?? ['_id', 1]
    all.sort((x, y) => ((x[sk] > y[sk] ? 1 : x[sk] < y[sk] ? -1 : 0)) * (dir as number))
    const page = all.slice(r.skip, r.skip + r.limit)
    return { docs: page.map(toDoc), more: all.length > r.skip + r.limit, took_ms: 3 }
  },
  mgCount: (a) => {
    const filter = parse(a.filter, 'The filter')
    const n = docs(a.db, a.coll).filter(d => matches(d, filter)).length
    return { n, estimate: !a.filter.trim() }
  },
  mgDocGet: (a) => {
    const id = unwrap(a.id)
    const d = docs(a.db, a.coll).find(x => x._id === id)
    if (!d) fail('db_refused', 'That document no longer exists.')
    return canonical(d)
  },
  mgDocPrepare: (a, { m }) => {
    const c = m.sqlConns.get(a.connId)
    const db = c && m.dbs.find(d => d.id === c.db)
    if (db?.write_mode === 'read_only') fail('db_read_only_mode', `${db.name} is read-only in Dalan. Nothing was sent.`)
    const op: Doc = a.op
    const id = op.kind === 'insert' ? null : unwrap(op.id)
    if (op.kind !== 'delete') parse(op.doc, 'The document')
    const cur = id === null ? undefined : docs(a.db, a.coll).find(d => d._id === id)
    const summary = op.kind === 'insert' ? `Insert 1 document into ${a.db}.${a.coll}.` : op.kind === 'replace' ? `Replace the document with _id ${id} in ${a.db}.${a.coll}.` : `Delete the document with _id ${id} from ${a.db}.${a.coll}. This can't be undone.`
    const pid = `mgp-${held.size + 1}-${Date.now()}`
    held.set(pid, { db: a.db, coll: a.coll, op })
    return { id: pid, db_name: db!.name, environment: db!.environment, summary, sample: cur ? canonical(cur) : null }
  },
  mgDocCommit: (a) => {
    const h = held.get(a.pendingId)
    if (!h) fail('db_confirm_needed', 'That confirmation was already used or isn\'t for this tab. Nothing was sent.')
    held.delete(a.pendingId)
    const list = docs(h.db, h.coll)
    if (h.op.kind === 'insert') {
      const d = parse(h.op.doc, 'The document')
      if (list.some(x => x._id === d._id)) fail('db_refused', `E11000 duplicate key error collection: ${h.db}.${h.coll} index: _id_ dup key: { _id: ${d._id} }`)
      list.push(d)
      return 1
    }
    const id = unwrap(h.op.id)
    const at = list.findIndex(d => d._id === id)
    if (at < 0) fail('db_refused', `That document no longer exists, so nothing was ${h.op.kind === 'delete' ? 'deleted' : 'replaced'}.`)
    if (h.op.kind === 'delete') list.splice(at, 1)
    else list[at] = { _id: id, ...parse(h.op.doc, 'The document') }
    return 1
  },
  mgDocDiscard: (a) => {
    held.delete(a.pendingId)
    return null
  },
}
