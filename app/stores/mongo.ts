import { defineStore } from 'pinia'
import type { TransferProgress, ShellReply, CodeLanguage, DbEvent_Serialize, MgAggResult, MgCount, MgDoc, MgDocOp, MgIndex, MgNode, MgPending_Serialize as MgPending, MgSchema, MgValidation, MgValidationCheck, PromptKind, RedisStepState, SqlServerInfo } from '~/types/bindings'
import { useTauri } from '~/composables/useTauri'

// MongoDB tabs (S44, F27): the connection like a SQL tab's, a tree of databases and collections, and one inner tab per
// open collection with the query bar, the three views and the document editor. Writes go through the gate: the window
// asks Rust to prepare a change, shows what it will do, and commits it once.

export type MongoStatus = 'connecting' | 'ready' | 'failed' | 'closed'
export interface MongoStep { label: string, note: string | null, detail: string | null, state: 'pending' | RedisStepState }
export interface DbNode { name: string, open: boolean, loading: boolean, colls: MgNode[] | null, error: string }
export interface Editor { mode: 'edit' | 'new', id: string | null, text: string, /** What the editor opened with, to tell unsaved typing from none. */ original: string, error: string }
export interface Review { pending: MgPending, error: string, busy: boolean, typed: string, /** What to refresh once it ran. */ then: (() => Promise<void>) | null, /** Where a failure shows when the view isn't the document list. */ errTo: { error: string } | null }
export interface Stage { id: number, op: string, body: string, enabled: boolean }
export interface AggState {
  stages: Stage[]
  /** The stage whose output is shown (its index). */
  selected: number
  sample: number
  hint: string
  result: MgAggResult | null
  loading: boolean
  error: string
  seq: number
  code: { language: CodeLanguage, text: string } | null
  viewName: string
}
export interface IndexForm { keys: string, name: string, unique: boolean, sparse: boolean, ttl: string, partial: string }
export interface SchemaState {
  sampleSize: number
  schema: MgSchema | null
  indexes: MgIndex[] | null
  validation: MgValidation | null
  /** The rules being edited (text), their level and action. */
  rules: string
  level: string
  action: string
  check: MgValidationCheck | null
  indexForm: IndexForm
  loading: boolean
  error: string
}
export const STAGE_OPS = ['$match', '$group', '$project', '$sort', '$limit', '$skip', '$unwind', '$lookup', '$addFields', '$count', '$sample', '$out', '$merge'] as const
export const STAGE_TEMPLATE: Record<string, string> = { $match: '{}', $group: '{ "_id": "$field", "n": { "$sum": 1 } }', $project: '{ "field": 1 }', $sort: '{ "field": 1 }', $limit: '10', $skip: '0', $unwind: '"$field"', $lookup: '{ "from": "other", "localField": "a", "foreignField": "b", "as": "joined" }', $addFields: '{ "total": { "$multiply": ["$price", "$qty"] } }', $count: '"n"', $sample: '{ "size": 10 }', $out: '"collection"', $merge: '{ "into": "collection" }' }
export interface CollTab {
  id: string
  db: string
  coll: string
  kind: string
  /** The query bar's text, applied only with Apply. */
  filter: string
  project: string
  sort: string
  collation: string
  applied: { filter: string, project: string, sort: string, collation: string }
  limit: number
  page: number
  view: 'list' | 'table' | 'json'
  docs: MgDoc[]
  more: boolean
  loading: boolean
  error: string
  tookMs: number | null
  count: MgCount | null
  selected: number | null
  editor: Editor | null
  review: Review | null
  section: 'documents' | 'aggregate' | 'schema'
  agg: AggState
  sch: SchemaState
  /** Which request is the latest: an older answer must not overwrite a newer page. */
  seq: number
  busy: boolean
}

/** One line of the shell's log: what was typed, and what came back (or the error). */
export interface ShellEntry { id: number, db: string, line: string, blocks: string[], note: string | null, error: string }
export interface ShellReview { pending: MgPending, line: string, typed: string, error: string, busy: boolean }
export const PAGE_LIMITS = [25, 50, 100, 200, 500]
const tabId = (db: string, coll: string) => `${db}.${coll}`

function setup() {
  const api = useTauri()
  const dbId = ref('')
  const connId = ref('')
  const status = ref<MongoStatus>('closed')
  const steps = ref<MongoStep[]>([])
  const prompt = ref<{ id: string, kind: PromptKind } | null>(null)
  const info = ref<SqlServerInfo | null>(null)
  const error = ref<{ code: string, message: string } | null>(null)
  const dbs = ref<DbNode[]>([])
  const treeFilter = ref('')
  const tabs = ref<CollTab[]>([])
  const activeTab = ref('')
  let generation = 0
  // ---- The shell (S47): one per tab, like the connection.
  /** The Transfer tab (S43): export a collection, back up and restore the database. */
  const transferOpen = ref(false)
  let transferCount = 0
  const shellOpen = ref(false)
  const shellDb = ref('')
  const shellLog = ref<ShellEntry[]>([])
  const shellInput = ref('')
  const shellHistory = ref<string[]>([])
  const shellBusy = ref(false)
  const shellReview = ref<ShellReview | null>(null
  )
  let shellCount = 0

  const message = (e: unknown, fallback: string) => (e instanceof Error ? e.message : (e as { message?: string })?.message ?? fallback)

  function onEvent(gen: number, e: DbEvent_Serialize) {
    if (gen !== generation) return
    switch (e.type) {
      case 'plan':
        steps.value = e.steps.map(s => ({ label: s.label, note: s.detail, detail: s.detail, state: 'pending' }))
        status.value = 'connecting'
        break
      case 'step': {
        const s = steps.value[e.index]
        if (s) Object.assign(s, { state: e.state, detail: e.detail ?? (e.state === 'active' ? s.detail : s.note) })
        break
      }
      case 'host':
        if (e.event.type === 'prompt') prompt.value = { id: e.event.prompt_id, kind: e.event.kind }
        break
      case 'ready':
        info.value = e.info
        status.value = 'ready'
        error.value = null
        prompt.value = null
        void loadDbs()
        break
      case 'failed':
        status.value = 'failed'
        error.value = { code: e.code, message: e.message }
        prompt.value = null
        break
      case 'closed':
        if (status.value !== 'failed') status.value = 'closed'
        break
    }
  }

  async function open(id: string) {
    const gen = ++generation
    if (connId.value) void api.dbClose(connId.value)
    dbId.value = id
    connId.value = ''
    status.value = 'connecting'
    steps.value = []
    prompt.value = null
    error.value = null
    info.value = null
    dbs.value = []
    tabs.value = []
    activeTab.value = ''
    shellOpen.value = false
    transferOpen.value = false
    shellLog.value = []
    shellReview.value = null
    try {
      const opened = await api.dbOpen(id, e => onEvent(gen, e))
      if (gen === generation) {
        connId.value = opened.conn_id
        if ((status.value as MongoStatus) === 'ready' && !dbs.value.length) void loadDbs()
      }
      else void api.dbClose(opened.conn_id)
    }
    catch (e) {
      if (gen !== generation) return
      status.value = 'failed'
      error.value = { code: (e as { code?: string }).code ?? 'internal', message: message(e, 'Couldn\'t open the database.') }
    }
  }
  const retry = () => (dbId.value ? open(dbId.value) : Promise.resolve())

  async function respond(accept: boolean, text: string | null = null, remember = false) {
    const p = prompt.value
    if (!p || !connId.value) return
    prompt.value = null
    await api.dbRespond(connId.value, p.id, { accept, text, remember }).catch(() => {})
  }

  function close() {
    generation++
    if (connId.value) void api.dbClose(connId.value)
    connId.value = ''
    status.value = 'closed'
  }

  // ---- The tree
  async function loadDbs() {
    if (!connId.value) return
    try {
      dbs.value = (await api.mgTree(connId.value, [])).map(o => ({ name: o.name, open: false, loading: false, colls: null, error: '' }))
      const first = dbs.value.find(d => d.name !== 'admin' && d.name !== 'local' && d.name !== 'config')
      if (first) await toggleDb(first, true)
    }
    catch (e) {
      error.value = { code: (e as { code?: string }).code ?? 'internal', message: message(e, 'Couldn\'t list the databases.') }
    }
  }
  async function toggleDb(d: DbNode, force?: boolean) {
    d.open = force ?? !d.open
    if (!d.open || d.colls || d.loading) return
    d.loading = true
    d.error = ''
    try {
      d.colls = await api.mgTree(connId.value, [d.name])
    }
    catch (e) {
      d.error = message(e, 'Couldn\'t list the collections.')
    }
    d.loading = false
  }

  // ---- Collections
  const current = computed(() => tabs.value.find(t => t.id === activeTab.value) ?? null)
  function openColl(db: string, c: MgNode) {
    const id = tabId(db, c.name)
    activeTab.value = id
    if (tabs.value.some(t => t.id === id)) return
    const blank = { filter: '', project: '', sort: '', collation: '' }
    const t: CollTab = { id, db, coll: c.name, kind: c.kind, ...blank, applied: { ...blank }, limit: 50, page: 0, view: 'list', docs: [], more: false, loading: false, error: '', tookMs: null, count: null, selected: null, editor: null, review: null, section: 'documents', seq: 0, busy: false, agg: { stages: [{ id: 1, op: '$match', body: '{}', enabled: true }], selected: 0, sample: 20, hint: '', result: null, loading: false, error: '', seq: 0, code: null, viewName: '' }, sch: { sampleSize: 200, schema: null, indexes: null, validation: null, rules: '', level: 'strict', action: 'error', check: null, indexForm: { keys: '', name: '', unique: false, sparse: false, ttl: '', partial: '' }, loading: false, error: '' } }
    tabs.value.push(t)
    void load(tabs.value.find(x => x.id === id)!)
  }
  function closeColl(id: string) {
    const i = tabs.value.findIndex(t => t.id === id)
    if (i < 0) return
    const t = tabs.value[i]!
    if (t.review) void api.mgDocDiscard(connId.value, t.review.pending.id)
    tabs.value.splice(i, 1)
    if (activeTab.value === id) activeTab.value = (tabs.value[i] ?? tabs.value[i - 1])?.id ?? ''
  }

  async function load(t: CollTab) {
    const seq = ++t.seq
    t.loading = true
    t.error = ''
    t.selected = null
    try {
      const a = t.applied
      const p = await api.mgFind(connId.value, { db: t.db, coll: t.coll, filter: a.filter, project: a.project, sort: a.sort, skip: t.page * t.limit, limit: t.limit, collation: a.collation })
      if (seq !== t.seq) return
      t.docs = p.docs
      t.more = p.more
      t.tookMs = p.took_ms
    }
    catch (e) {
      if (seq !== t.seq) return
      t.docs = []
      t.more = false
      t.error = message(e, 'Couldn\'t read the documents.')
    }
    t.loading = false
  }
  /** Apply: the query bar's text becomes the query; the count follows it. */
  async function apply(t: CollTab) {
    t.applied = { filter: t.filter, project: t.project, sort: t.sort, collation: t.collation }
    t.page = 0
    t.count = null
    await load(t)
  }
  async function setLimit(t: CollTab, n: number) {
    t.limit = n
    t.page = 0
    await load(t)
  }
  async function nextPage(t: CollTab) {
    if (!t.more) return
    t.page++
    await load(t)
  }
  async function prevPage(t: CollTab) {
    if (t.page === 0) return
    t.page--
    await load(t)
  }
  async function countDocs(t: CollTab) {
    try {
      t.count = await api.mgCount(connId.value, t.db, t.coll, t.applied.filter)
    }
    catch (e) {
      t.error = message(e, 'Couldn\'t count the documents.')
    }
  }

  // ---- Documents: edit, insert, delete (each goes through the gate)
  /** The editor opens on the whole document from the server (never on a projected or cut copy). */
  async function startEdit(t: CollTab, d: MgDoc) {
    try {
      const text = await api.mgDocGet(connId.value, t.db, t.coll, d.id)
      t.editor = { mode: 'edit', id: d.id, text, original: text, error: '' }
    }
    catch (e) {
      t.error = message(e, 'Couldn\'t read that document.')
    }
  }
  function startNew(t: CollTab) {
    t.editor = { mode: 'new', id: null, text: '{\n  \n}', original: '{\n  \n}', error: '' }
  }
  async function hold(t: CollTab, op: MgDocOp): Promise<boolean> {
    return holdWith(t, () => api.mgDocPrepare(connId.value, t.db, t.coll, op), null)
  }
  /** Asks Rust to hold a change and shows its review. `then` refreshes whatever changed once it ran. */
  async function holdWith(t: CollTab, prepare: () => Promise<MgPending>, then: (() => Promise<void>) | null, errorTo?: { error: string }): Promise<boolean> {
    if (t.busy || t.review) return false
    t.busy = true
    try {
      t.review = { pending: await prepare(), error: '', busy: false, typed: '', then, errTo: errorTo ?? null }
      return true
    }
    catch (e) {
      const msg = message(e, 'Couldn\'t prepare that change.')
      if (errorTo) errorTo.error = msg
      else if (t.editor) t.editor.error = msg
      else t.error = msg
      return false
    }
    finally {
      t.busy = false
    }
  }
  /** Save in the editor: the change is held and shown before anything is sent. */
  async function saveEditor(t: CollTab) {
    const ed = t.editor
    if (!ed) return
    ed.error = ''
    await hold(t, ed.mode === 'new' ? { kind: 'insert', doc: ed.text } : { kind: 'replace', id: ed.id!, doc: ed.text })
  }
  async function deleteDoc(t: CollTab, d: MgDoc) {
    await hold(t, { kind: 'delete', id: d.id })
  }
  async function commitReview(t: CollTab) {
    const r = t.review
    if (!r || r.busy) return
    r.busy = true
    r.error = ''
    try {
      await api.mgDocCommit(connId.value, r.pending.id, r.pending.level === 'typed' ? r.typed : null)
      t.review = null
      t.editor = null
      if (r.then) {
        await r.then()
        return
      }
      t.count = null
      await load(t)
      // The last document of a later page was removed: step back to a page that has documents.
      if (!t.docs.length && t.page > 0 && !t.error) {
        t.page--
        await load(t)
      }
    }
    catch (e) {
      r.error = message(e, 'The change wasn\'t made.')
      // The held change is used up whether it ran or not: the review ends, the editor stays for another try.
      // A wrong typed name keeps the review (the change is still held); anything else used it up.
      if (r.pending.level === 'typed' && (e as { code?: string }).code === 'db_confirm_needed' && r.typed !== r.pending.db_name) {
        r.busy = false
        return
      }
      t.review = null
      if (r.errTo) r.errTo.error = r.error
      else if (t.editor) t.editor.error = r.error
      else t.error = r.error
    }
  }
  function cancelReview(t: CollTab) {
    const r = t.review
    t.review = null
    if (r) void api.mgDocDiscard(connId.value, r.pending.id)
  }

  // ---- Aggregation (S45)
  let stageCount = 1
  const request = (t: CollTab, upto: number | null) => ({ db: t.db, coll: t.coll, stages: t.agg.stages.map(s => ({ op: s.op, body: s.body, enabled: s.enabled })), upto, sample: t.agg.sample, hint: t.agg.hint })
  async function aggRun(t: CollTab) {
    const a = t.agg
    const seq = ++a.seq
    a.loading = true
    a.error = ''
    try {
      const r = await api.mgAggregate(connId.value, request(t, a.selected))
      if (seq === a.seq) a.result = r
    }
    catch (e) {
      if (seq !== a.seq) return
      a.result = null
      a.error = message(e, 'The pipeline didn\'t run.')
    }
    if (seq === a.seq) a.loading = false
  }
  function aggAdd(t: CollTab, op: string) {
    t.agg.stages.push({ id: ++stageCount, op, body: STAGE_TEMPLATE[op] ?? '{}', enabled: true })
    t.agg.selected = t.agg.stages.length - 1
  }
  function aggRemove(t: CollTab, i: number) {
    t.agg.stages.splice(i, 1)
    t.agg.selected = Math.max(0, Math.min(t.agg.selected, t.agg.stages.length - 1))
  }
  function aggMove(t: CollTab, i: number, d: -1 | 1) {
    const s = t.agg.stages
    const j = i + d
    if (j < 0 || j >= s.length) return
    ;[s[i], s[j]] = [s[j]!, s[i]!]
    t.agg.selected = j
  }
  const aggWrite = (t: CollTab) => holdWith(t, () => api.mgAggPrepare(connId.value, request(t, null)), async () => { await aggRun(t) }, t.agg)
  const aggView = (t: CollTab) => holdWith(t, () => api.mgViewPrepare(connId.value, t.db, t.coll, t.agg.viewName, request(t, null).stages), async () => { t.agg.viewName = '' }, t.agg)
  async function aggCode(t: CollTab, language: CodeLanguage) {
    try {
      t.agg.code = { language, text: await api.mgPipelineCode(t.coll, request(t, null).stages, language) }
    }
    catch (e) {
      t.agg.error = message(e, 'Couldn\'t make the code.')
    }
  }

  // ---- Schema, indexes, validation (S46)
  async function loadSchema(t: CollTab) {
    const s = t.sch
    s.loading = true
    s.error = ''
    try {
      s.schema = await api.mgSchema(connId.value, t.db, t.coll, s.sampleSize)
    }
    catch (e) {
      s.error = message(e, 'Couldn\'t sample the collection.')
    }
    s.loading = false
  }
  async function loadIndexes(t: CollTab) {
    try {
      t.sch.indexes = await api.mgIndexes(connId.value, t.db, t.coll)
    }
    catch (e) {
      t.sch.error = message(e, 'Couldn\'t list the indexes.')
    }
  }
  async function loadValidation(t: CollTab) {
    try {
      const v = await api.mgValidation(connId.value, t.db, t.coll)
      Object.assign(t.sch, { validation: v, rules: v.validator, level: v.level, action: v.action, check: null })
    }
    catch (e) {
      t.sch.error = message(e, 'Couldn\'t read the validation rules.')
    }
  }
  /** Opening the Schema section reads all three. */
  async function openSchema(t: CollTab) {
    t.section = 'schema'
    if (t.sch.loading) return
    t.sch.error = ''
    // Whatever failed to load last time is asked for again.
    await Promise.all([t.sch.schema ? null : loadSchema(t), t.sch.indexes ? null : loadIndexes(t), t.sch.validation ? null : loadValidation(t)])
  }
  async function checkRules(t: CollTab) {
    t.sch.error = ''
    try {
      t.sch.check = await api.mgValidationCheck(connId.value, t.db, t.coll, t.sch.rules)
    }
    catch (e) {
      t.sch.error = message(e, 'Couldn\'t check the rules.')
    }
  }
  const saveRules = (t: CollTab) => holdWith(t, () => api.mgValidationPrepare(connId.value, t.db, t.coll, t.sch.rules, t.sch.level, t.sch.action), () => loadValidation(t), t.sch)
  const createIndex = async (t: CollTab) => {
    const f = t.sch.indexForm
    t.sch.error = ''
    if (f.ttl.trim() && !/^\d+$/.test(f.ttl.trim())) {
      t.sch.error = 'TTL is a whole number of seconds, like 3600.'
      return false
    }
    return holdWith(t, () => api.mgIndexPrepare(connId.value, t.db, t.coll, { kind: 'create', keys: f.keys, name: f.name, unique: f.unique, sparse: f.sparse, ttl_seconds: f.ttl.trim() ? Number(f.ttl) : null, partial: f.partial }), async () => { t.sch.indexForm = { keys: '', name: '', unique: false, sparse: false, ttl: '', partial: '' }; await loadIndexes(t) }, t.sch)
  }
  const dropIndex = (t: CollTab, name: string) => holdWith(t, () => api.mgIndexPrepare(connId.value, t.db, t.coll, { kind: 'drop', name }), () => loadIndexes(t), t.sch)

  // ---- Transfer (S43)
  function openTransfer() {
    transferOpen.value = true
    activeTab.value = 'transfer'
  }
  function closeTransfer() {
    transferOpen.value = false
    if (activeTab.value === 'transfer') activeTab.value = tabs.value.at(-1)?.id ?? ''
  }
  const newTransferId = () => `t${++transferCount}-${Date.now()}`
  const stopTransfer = (id: string) => api.sqlTransferStop(id).catch(() => {})
  const exportColl = (id: string, db: string, coll: string, filter: string, onProgress: (p: TransferProgress) => void) => api.mgExport(connId.value, id, db, coll, filter, onProgress)
  const backupCheck = () => api.sqlBackupCheck(connId.value)
  const backupStart = (id: string, onProgress: (p: TransferProgress) => void) => api.sqlBackupStart(connId.value, id, onProgress)

  // ---- The shell
  function openShell() {
    shellOpen.value = true
    activeTab.value = 'shell'
    if (!shellDb.value) shellDb.value = info.value?.database && info.value.database !== 'admin' ? info.value.database : (dbs.value.find(d => !['admin', 'local', 'config'].includes(d.name))?.name ?? 'admin')
  }
  function closeShell() {
    shellOpen.value = false
    if (shellReview.value) void api.mgDocDiscard(connId.value, shellReview.value.pending.id)
    shellReview.value = null
    if (activeTab.value === 'shell') activeTab.value = tabs.value.at(-1)?.id ?? ''
  }
  const log = (line: string, db: string, r: Partial<ShellEntry>) => shellLog.value.push({ id: ++shellCount, db, line, blocks: [], note: null, error: '', ...r })
  async function shellRun() {
    const line = shellInput.value.trim()
    if (!line || shellBusy.value || shellReview.value) return
    shellInput.value = ''
    if (shellHistory.value.at(-1) !== line) shellHistory.value.push(line)
    shellBusy.value = true
    const db = shellDb.value
    try {
      const r = await api.mgShell(connId.value, db, line)
      shellDb.value = r.db
      if (r.pending) shellReview.value = { pending: r.pending, line, typed: '', error: '', busy: false }
      else log(line, db, { blocks: r.reply?.blocks ?? [], note: r.reply?.note ?? null })
    }
    catch (e) {
      log(line, db, { error: message(e, 'The command failed.') })
    }
    shellBusy.value = false
  }
  async function shellCommit() {
    const r = shellReview.value
    if (!r || r.busy) return
    r.busy = true
    r.error = ''
    try {
      const reply: ShellReply = await api.mgShellCommit(connId.value, r.pending.id, r.pending.level === 'typed' ? r.typed : null)
      log(r.line, shellDb.value, { blocks: reply.blocks, note: reply.note })
      shellReview.value = null
      // Collections and counts may have changed under an open tab.
      for (const t of tabs.value) void load(t)
    }
    catch (e) {
      const msg = message(e, 'The change wasn\'t made.')
      // A wrong typed name keeps the dialog (the change is still held); anything else used it up.
      if (r.pending.level === 'typed' && (e as { code?: string }).code === 'db_confirm_needed' && r.typed !== r.pending.db_name) {
        r.error = msg
        r.busy = false
        return
      }
      log(r.line, shellDb.value, { error: msg })
      shellReview.value = null
    }
  }
  function shellCancel() {
    const r = shellReview.value
    shellReview.value = null
    if (r) {
      void api.mgDocDiscard(connId.value, r.pending.id)
      log(r.line, shellDb.value, { error: 'Cancelled. Nothing was sent.' })
    }
  }

  return { transferOpen, openTransfer, closeTransfer, newTransferId, stopTransfer, exportColl, backupCheck, backupStart, shellOpen, shellDb, shellLog, shellInput, shellHistory, shellBusy, shellReview, openShell, closeShell, shellRun, shellCommit, shellCancel, aggRun, aggAdd, aggRemove, aggMove, aggWrite, aggView, aggCode, loadSchema, loadIndexes, loadValidation, openSchema, checkRules, saveRules, createIndex, dropIndex, dbId, connId, status, steps, prompt, info, error, dbs, treeFilter, tabs, activeTab, current, open, retry, respond, close, toggleDb, openColl, closeColl, load, apply, setLimit, nextPage, prevPage, countDocs, startEdit, startNew, saveEditor, deleteDoc, commitReview, cancelReview }
}

const makeStore = (key: string) => defineStore(`mongo:${key}`, setup)
const tabStores = new Map<string, ReturnType<typeof makeStore>>()

export function useMongoStore(tabKey: string) {
  if (!tabStores.has(tabKey)) tabStores.set(tabKey, makeStore(tabKey))
  return tabStores.get(tabKey)!()
}

export function dropMongoStore(tabKey: string) {
  if (!tabStores.has(tabKey)) return
  useMongoStore(tabKey).close()
  tabStores.delete(tabKey)
}

/** Vault locked: Rust already closed every connection; forget the tabs' state. */
export function clearAllMongo() {
  for (const key of [...tabStores.keys()]) dropMongoStore(key)
}
