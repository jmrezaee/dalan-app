// MongoDB tabs (replica/databases/spec.md D8): two databases, `shop` (orders with 600 documents, customers with 30, an
// `open_orders` view) and `admin`. Filters are exact matches on top-level fields; every other query shape is the
// real backend's job. Edits change the fake's documents so a test can see them come back.
import { fail, type Handlers } from '../types'

type Keys = 'mgTree' | 'mgFind' | 'mgCount' | 'mgDocGet' | 'mgDocPrepare' | 'mgDocCommit' | 'mgDocDiscard' | 'mgAggregate' | 'mgAggPrepare' | 'mgViewPrepare' | 'mgPipelineCode' | 'mgSchema' | 'mgIndexes' | 'mgIndexPrepare' | 'mgValidation' | 'mgValidationCheck' | 'mgValidationPrepare'
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
/** Changes held for the schema views: what to do when confirmed. */
const heldRun = new Map<string, () => void>()
const indexes: Record<string, { name: string, keys: string, unique: boolean, uses: number }[]> = {}
const rules: Record<string, { validator: string, level: string, action: string }> = {}
const ixOf = (db: string, coll: string) => (indexes[`${db}.${coll}`] ??= [{ name: '_id_', keys: '{"_id":1}', unique: true, uses: 12 }, { name: 'status_1', keys: '{"status":1}', unique: false, uses: 40 }])
const hold = (m: any, connId: string, summary: string, level: 'confirm' | 'typed', run: () => void, sample: string | null = null) => {
  const c = m.sqlConns.get(connId)
  const db = c && m.dbs.find((d: any) => d.id === c.db)
  if (db?.write_mode === 'read_only') fail('db_read_only_mode', `${db.name} is read-only in Dalan. Nothing was sent.`)
  const id = `mgs-${heldRun.size + 1}-${Date.now()}`
  heldRun.set(id, run)
  return { id, db_name: db!.name, environment: db!.environment, summary, sample, level }
}
const parseJson = (t: string, what: string): any => {
  try { return t.trim() ? JSON.parse(t) : {} }
  catch (e) { return fail('validation', `${what} isn't valid JSON: ${(e as Error).message}`) }
}
function runPipeline(db: string, coll: string, stages: any[], upto: number | null) {
  let out: Doc[] = docs(db, coll).slice()
  let stopped = false
  for (const [i, st] of stages.entries()) {
    if (upto !== null && i > upto) break
    if (!st.enabled) continue
    if (st.op === '$out' || st.op === '$merge') { stopped = true; break }
    const arg = parseJson(st.body, `The ${st.op} stage`)
    if (st.op === '$match') out = out.filter(d => matches(d, arg))
    else if (st.op === '$limit') out = out.slice(0, Number(arg))
    else if (st.op === '$skip') out = out.slice(Number(arg))
    else if (st.op === '$sort') { const [k, dir] = Object.entries(arg)[0] ?? ['_id', 1]; out.sort((x, y) => (x[k] > y[k] ? 1 : x[k] < y[k] ? -1 : 0) * (dir as number)) }
    else if (st.op === '$count') out = [{ [arg]: out.length }]
    else if (st.op === '$group') {
      const key = String(arg._id).replace(/^\$/, '')
      const g = new Map<string, number>()
      for (const d of out) g.set(JSON.stringify(d[key]), (g.get(JSON.stringify(d[key])) ?? 0) + 1)
      out = [...g.entries()].map(([k, n]) => ({ _id: JSON.parse(k), n }))
    }
    else fail('db_refused', `Unrecognized pipeline stage name: '${st.op}'`)
  }
  return { out, stopped }
}

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
    return { id: pid, db_name: db!.name, environment: db!.environment, summary, sample: cur ? canonical(cur) : null, level: 'confirm' }
  },
  mgDocCommit: (a) => {
    const run = heldRun.get(a.pendingId)
    if (run) {
      const pend = a.pendingId as string
      const typedNeeded = pend.endsWith('-typed')
      if (typedNeeded && a.typedName !== 'docs-prod') fail('db_confirm_needed', 'The name doesn\'t match the database\'s name. Nothing was sent.')
      heldRun.delete(pend)
      run()
      return 1
    }
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
  mgAggregate: (a) => {
    const r = a.request
    const { out, stopped } = runPipeline(r.db, r.coll, r.stages, r.upto)
    return { docs: out.slice(0, r.sample).map(toDoc), more: out.length > r.sample, took_ms: 4, stopped_at_write: stopped }
  },
  mgAggPrepare: (a, { m }) => {
    const out = a.request.stages.find((s: any) => s.enabled && (s.op === '$out' || s.op === '$merge'))
    if (!out) fail('validation', 'This pipeline doesn\'t end in $out or $merge, so there is nothing to confirm. Run it to see the output.')
    return hold(m, a.connId, `Run the pipeline on ${a.request.db}.${a.request.coll} and ${out.op === '$out' ? 'replace the collection with its output' : 'merge its output'}.`, 'confirm', () => {})
  },
  mgViewPrepare: (a, { m }) => {
    if (!a.name.trim()) fail('validation', 'Give the view a name without $ and not starting with system.')
    return hold(m, a.connId, `Create the view ${a.name} on ${a.db}.${a.coll} with ${a.stages.length} stage${a.stages.length === 1 ? '' : 's'}.`, 'confirm', () => {})
  },
  mgPipelineCode: (a) => {
    const stages = a.stages.filter((s: any) => s.enabled).map((s: any) => ({ [s.op]: parseJson(s.body, `The ${s.op} stage`) }))
    const json = JSON.stringify(stages, null, 2)
    return a.language === 'json' ? json : a.language === 'node' ? `const pipeline = EJSON.parse(\`${json}\`)\nawait db.collection("${a.coll}").aggregate(pipeline).toArray()` : a.language === 'python' ? `pipeline = json_util.loads("""${json}""")\nlist(db["${a.coll}"].aggregate(pipeline))` : `db.getCollection("${a.coll}").aggregate(EJSON.parse(\`${json}\`)).toArray()`
  },
  mgSchema: (a) => {
    const list = docs(a.db, a.coll)
    const fields = new Map<string, { present: number, types: Map<string, number>, example: string | null }>()
    for (const d of list) for (const [k, v] of Object.entries(d)) {
      const f = fields.get(k) ?? { present: 0, types: new Map(), example: null }
      f.present++
      const t = v === null ? 'null' : typeof v === 'number' ? 'int' : typeof v === 'boolean' ? 'bool' : typeof v === 'object' ? 'object' : 'string'
      f.types.set(t, (f.types.get(t) ?? 0) + 1)
      f.example ??= typeof v === 'object' ? null : JSON.stringify(v)
      fields.set(k, f)
    }
    return { sampled: list.length, truncated: false, fields: [...fields.entries()].map(([path, f]) => ({ path, present: f.present, types: [...f.types.entries()].map(([name, count]) => ({ name, count })), example: f.example })) }
  },
  mgIndexes: (a) => ixOf(a.db, a.coll).map(i => ({ name: i.name, keys: i.keys, unique: i.unique, sparse: false, ttl_seconds: null, partial: null, uses: i.uses, since_ms: 1_760_000_000_000 })),
  mgIndexPrepare: (a, { m }) => {
    const op = a.op
    if (op.kind === 'create') {
      const keys = parseJson(op.keys, 'The index keys')
      if (!Object.keys(keys).length) fail('validation', 'An index needs at least one key, like { "status": 1 }.')
      const name = op.name.trim() || Object.entries(keys).map(([k, v]) => `${k}_${v}`).join('_')
      return hold(m, a.connId, `Create the index ${name} on ${a.coll}.`, 'confirm', () => { if (ixOf(a.db, a.coll).some(i => i.name === name)) fail('db_refused', `Index already exists with a different name: ${name}`); ixOf(a.db, a.coll).push({ name, keys: JSON.stringify(keys), unique: op.unique, uses: 0 }) })
    }
    if (op.name === '_id_') fail('validation', 'The _id index can\'t be dropped.')
    const p = hold(m, a.connId, `Drop the index ${op.name} on ${a.coll}. Queries that used it get slower.`, 'typed', () => { indexes[`${a.db}.${a.coll}`] = ixOf(a.db, a.coll).filter(i => i.name !== op.name) })
    heldRun.set(`${p.id}-typed`, heldRun.get(p.id)!)
    heldRun.delete(p.id)
    return { ...p, id: `${p.id}-typed` }
  },
  mgValidation: (a) => ({ validator: rules[`${a.db}.${a.coll}`]?.validator ?? '', level: rules[`${a.db}.${a.coll}`]?.level ?? 'strict', action: rules[`${a.db}.${a.coll}`]?.action ?? 'error' }),
  mgValidationCheck: (a) => {
    parseJson(a.validator, 'The rules')
    const bad = a.validator.trim() ? docs(a.db, a.coll).slice(0, 3) : []
    return { failing: a.validator.trim() ? 3 : 0, sample: bad.map(toDoc) }
  },
  mgValidationPrepare: (a, { m }) => {
    parseJson(a.validator, 'The rules')
    return hold(m, a.connId, a.validator.trim() ? `Change the validation rules of ${a.coll} (level ${a.level}, action ${a.action}). Existing documents are not touched.` : `Remove the validation rules of ${a.coll}.`, 'confirm', () => { rules[`${a.db}.${a.coll}`] = { validator: a.validator, level: a.level, action: a.action } })
  },
}
