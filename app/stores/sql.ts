import { defineStore } from 'pinia'
import type { Activity_Serialize as Activity, ActivityView, CsvPreview, SessionPending, SessionAction, ExportFormat, TransferProgress, Diagram, ExplainPlan_Serialize as ExplainPlan, StructureOp_Deserialize as StructureOp, StructurePending, TableStructure_Serialize as TableStructure, PendingRun, TxnState, GridPending, SqlCatalog, SqlRunEvent_Serialize, RunMode, SqlCell_Serialize as Cell, DbEvent_Serialize, PageCursor_Serialize, PromptKind, RedisStepState, SqlColumn_Serialize as SqlColumn, SqlObject_Serialize as SqlObject, SqlServerInfo, SqlTableInfo_Serialize as SqlTableInfo } from '~/types/bindings'
import { useTauri } from '~/composables/useTauri'

export type SqlStatus = 'connecting' | 'ready' | 'failed' | 'closed'
export interface SqlStep { label: string, note: string | null, detail: string | null, state: 'pending' | RedisStepState }

/** One schema of the object tree. Its tables are listed when it is first opened. */
export interface SchemaNode { name: string, open: boolean, loading: boolean, objects: SqlObject[] | null, error: string }

/** An open table (S36). Pages are fetched one at a time; `cursors[n]` is where page `n` starts. */
export interface TableTab {
  id: string
  schema: string
  table: string
  kind: SqlObject['kind']
  info: SqlTableInfo | null
  /** What is in the Where box (not applied until Apply). */
  filter: string
  applied: string
  sort: { column: string, descending: boolean }[]
  /** The columns shown; empty is all of them. */
  columns: string[]
  cols: SqlColumn[]
  rows: Cell[][]
  cursors: (PageCursor_Serialize | null)[]
  page: number
  next: PageCursor_Serialize | null
  keyset: boolean
  loading: boolean
  error: string
  elapsedMs: number | null
  estimate: number | null
  exact: number | null
  selected: { row: number, col: number } | null
  /** The whole value of the selected cell, when the page cut it. */
  full: { text: string | null, loading: boolean } | null
  /** Edits not yet committed (F22): `row:col` to the new text (`null` is NULL), and rows marked for deletion. */
  edits: Record<string, string | null>
  deleted: number[]
  /** The commit review the gate holds, and what went wrong with it. */
  review: { pending: GridPending, error: string, busy: boolean } | null
  /** Which view of the table shows (S38): the data, or its structure. */
  view: 'data' | 'structure' | 'indexes' | 'keys' | 'triggers' | 'ddl'
  structure: TableStructure | null
  structureError: string
  /** Changes waiting for Review and apply (the typed diff), and the review the gate holds. */
  ops: StructureOp[]
  structureReview: { pending: StructurePending, error: string, busy: boolean } | null
}

/** The plan of the statement last explained in a query tab (S39). */
export interface PlanState { state: 'loading' | 'ready' | 'failed', analyze: boolean, plan: ExplainPlan | null, error: string, statement: string }
/** A diagram tab (S40). */
export interface DiagramTab { id: string, title: string, start: string[], steps: number, keysOnly: boolean, data: Diagram | null, loading: boolean, error: string }

/** One statement's answer in the query editor (S37). Rows arrive in batches while `state` is `running`. */
export interface ResultSet {
  index: number
  statement: string
  cols: SqlColumn[]
  rows: Cell[][]
  state: 'running' | 'done' | 'failed'
  /** The row limit cut the result off. */
  more: boolean
  elapsedMs: number | null
  selected: { row: number, col: number } | null
}
export interface QueryMessage { kind: 'info' | 'error' | 'refused', text: string, statement?: string }
export type QueryView = `r${number}` | 'messages' | 'history' | 'saved' | 'explain'
/** A query tab (S37). It keeps its text, results and messages while another tab shows. */
export interface QueryTab {
  id: string
  title: string
  text: string
  /** The text as last saved or loaded; a different `text` shows the unsaved dot. */
  saved: string
  savedId: string | null
  cursor: number
  running: boolean
  results: ResultSet[]
  messages: QueryMessage[]
  view: QueryView
  limit: number
  timeoutS: number
  totalMs: number | null
  /** Manual transactions: each run happens inside one that stays open until Commit or Roll back. */
  manual: boolean
  txn: TxnState | null
  /** A change the gate holds for the person's answer (S42). */
  pending: PendingRun | null
  plan: PlanState | null
  /** The tab's × was pressed with a transaction open: asking what to do with it. */
  closeAsk: boolean
}

export const PAGE_SIZE = 200
const tableId = (schema: string, table: string) => `${schema}.${table}`

function setup() {
  const api = useTauri()
  const dbId = ref('')
  /** Rust's connection id. Empty until `db_open` answers; events can arrive before that. */
  const connId = ref('')
  const status = ref<SqlStatus>('closed')
  const steps = ref<SqlStep[]>([])
  const prompt = ref<{ id: string, kind: PromptKind } | null>(null)
  const info = ref<SqlServerInfo | null>(null)
  const error = ref<{ code: string, message: string } | null>(null)
  const schemas = ref<SchemaNode[]>([])
  const treeFilter = ref('')
  const tabs = ref<TableTab[]>([])
  const activeTab = ref('')
  const current = computed(() => tabs.value.find(t => t.id === activeTab.value) ?? null)
  let generation = 0
  const queries = ref<QueryTab[]>([])
  let queryCount = 0
  /** Tables and columns for completion; read when the first editor opens. */
  const catalog = ref<SqlCatalog | null>(null)
  const history = ref<string[]>([])
  const diagrams = ref<DiagramTab[]>([])
  /** The Activity and Transfer inner tabs (S41, S43): each is open or not, one of each. */
  const special = ref<('activity' | 'transfer')[]>([])
  let diagramCount = 0

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
        void loadSchemas()
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

  /** Opens (or re-opens) the tab's connection. Progress arrives as events. */
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
    schemas.value = []
    tabs.value = []
    queries.value = []
    diagrams.value = []
    special.value = []
    catalog.value = null
    activeTab.value = ''
    try {
      const opened = await api.dbOpen(id, e => onEvent(gen, e))
      if (gen === generation) {
        connId.value = opened.conn_id
        if ((status.value as SqlStatus) === 'ready' && !schemas.value.length) void loadSchemas()
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
  async function loadSchemas() {
    if (!connId.value) return
    try {
      const list = await api.sqlObjects(connId.value, [])
      schemas.value = list.map(o => ({ name: o.name, open: false, loading: false, objects: null, error: '' }))
      // The usual one is open from the start.
      const first = schemas.value.find(s => s.name === 'public') ?? schemas.value[0]
      if (first) await toggleSchema(first, true)
    }
    catch (e) {
      error.value = { code: (e as { code?: string }).code ?? 'internal', message: message(e, 'Couldn\'t read the schemas.') }
    }
  }

  async function toggleSchema(s: SchemaNode, force?: boolean) {
    s.open = force ?? !s.open
    if (!s.open || s.objects || s.loading) return
    s.loading = true
    s.error = ''
    try {
      s.objects = await api.sqlObjects(connId.value, [s.name])
    }
    catch (e) {
      s.error = message(e, 'Couldn\'t list the tables.')
    }
    s.loading = false
  }

  // ---- Tables
  async function openTable(schema: string, o: SqlObject) {
    const id = tableId(schema, o.name)
    const existing = tabs.value.find(t => t.id === id)
    if (existing) {
      activeTab.value = id
      return
    }
    const tab = reactive<TableTab>({
      id, schema, table: o.name, kind: o.kind, info: null, filter: '', applied: '', sort: [], columns: [], cols: [], rows: [], cursors: [null], page: 0, next: null,
      keyset: false, loading: false, error: '', elapsedMs: null, estimate: o.estimate ?? null, exact: null, selected: null, full: null, edits: {}, deleted: [], review: null, view: 'data', structure: null, structureError: '', ops: [], structureReview: null,
    })
    tabs.value.push(tab)
    activeTab.value = id
    try {
      tab.info = await api.sqlTableInfo(connId.value, schema, o.name)
    }
    catch (e) {
      tab.error = message(e, 'Couldn\'t read this table.')
      return
    }
    await loadPage(tab, 0)
  }

  function closeTable(id: string) {
    const i = tabs.value.findIndex(t => t.id === id)
    if (i < 0) return
    tabs.value.splice(i, 1)
    if (activeTab.value === id) activeTab.value = tabs.value[Math.min(i, tabs.value.length - 1)]?.id ?? ''
  }

  const changeCount = (tab: TableTab) => Object.keys(tab.edits).length + tab.deleted.length

  // An answer that comes back after a newer request for the same table is dropped.
  const runs = new Map<string, number>()

  async function loadPage(tab: TableTab, page: number) {
    if (!connId.value) return
    const run = (runs.get(tab.id) ?? 0) + 1
    runs.set(tab.id, run)
    tab.loading = true
    tab.error = ''
    try {
      const res = await api.sqlTablePage(connId.value, {
        schema: tab.schema, table: tab.table, filter: tab.applied, sort: tab.sort, columns: tab.columns,
        cursor: tab.cursors[page] ?? null, page_size: PAGE_SIZE,
      })
      if (runs.get(tab.id) !== run) return
      tab.cols = res.columns
      tab.rows = res.rows
      tab.page = page
      tab.next = res.next
      tab.keyset = res.keyset
      tab.elapsedMs = res.elapsed_ms
      tab.cursors[page + 1] = res.next
      tab.cursors.length = page + 2
      tab.selected = null
      tab.full = null
      tab.edits = {}
      tab.deleted = []
    }
    catch (e) {
      if (runs.get(tab.id) !== run) return
      tab.error = message(e, 'Couldn\'t read this table.')
    }
    if (runs.get(tab.id) === run) tab.loading = false
  }

  /**
   * Apply: the Where box, the sort and the columns start the table again from its first page. If the server or the
   * filter check refuses, nothing is kept: the table stays on the page, filter, sort and columns it had.
   */
  async function apply(tab: TableTab, change: { sort?: { column: string, descending: boolean }[], columns?: string[] } = {}) {
    if (changeCount(tab) > 0) {
      tab.error = 'Commit or discard your changes first: loading rows again would lose them.'
      return
    }
    const before = { applied: tab.applied, sort: tab.sort, columns: tab.columns, cursors: [...tab.cursors], exact: tab.exact }
    tab.applied = tab.filter.trim()
    if (change.sort) tab.sort = change.sort
    if (change.columns) tab.columns = change.columns
    tab.exact = null
    tab.cursors = [null]
    await loadPage(tab, 0)
    if (tab.error) Object.assign(tab, before)
  }
  const refresh = (tab: TableTab) => loadPage(tab, tab.page)
  const nextPage = (tab: TableTab) => (tab.next ? loadPage(tab, tab.page + 1) : Promise.resolve())
  const prevPage = (tab: TableTab) => (tab.page > 0 ? loadPage(tab, tab.page - 1) : Promise.resolve())

  /** Click on a column header: ascending, then descending, then off. */
  function sortBy(tab: TableTab, column: string) {
    const cur = tab.sort[0]
    return apply(tab, { sort: !cur || cur.column !== column ? [{ column, descending: false }] : cur.descending ? [] : [{ column, descending: true }] })
  }

  async function countExactly(tab: TableTab) {
    try {
      tab.exact = await api.sqlCount(connId.value, tab.schema, tab.table, tab.applied)
    }
    catch (e) {
      tab.error = message(e, 'Couldn\'t count the rows.')
    }
  }

  /** The primary key of a row as (column, text) pairs; empty when the table has none. */
  function keyOf(tab: TableTab, row: number) {
    const pk = (tab.info?.columns ?? []).filter(c => c.pk_position !== null).sort((a, b) => (a.pk_position ?? 0) - (b.pk_position ?? 0))
    const out: { column: string, value: string }[] = []
    for (const k of pk) {
      const i = tab.cols.findIndex(c => c.name === k.name)
      const v = i >= 0 ? tab.rows[row]?.[i]?.v : null
      // The key column isn't shown (a column subset), so the row can't be picked out.
      if (v == null) return []
      out.push({ column: k.name, value: v })
    }
    return out
  }

  /** Selecting a cell; a cut value is fetched whole for the panel. */
  async function select(tab: TableTab, row: number, col: number) {
    tab.selected = { row, col }
    tab.full = null
    const cell = tab.rows[row]?.[col]
    if (!cell?.cut) return
    const key = keyOf(tab, row)
    if (!key.length) return
    tab.full = { text: null, loading: true }
    try {
      const whole = await api.sqlCell(connId.value, tab.schema, tab.table, key, tab.cols[col]!.name)
      if (tab.selected?.row === row && tab.selected.col === col) tab.full = { text: whole.v, loading: false }
    }
    catch {
      tab.full = null
    }
  }

  // ---- Editing (F22): the changes stay in the grid until the commit review

  function setCell(tab: TableTab, row: number, col: number, value: string | null) {
    const key = `${row}:${col}`
    // Back to what the server sent: nothing to commit for this cell.
    if ((tab.rows[row]?.[col]?.v ?? null) === value) {
      const { [key]: _same, ...rest } = tab.edits
      tab.edits = rest
    }
    else tab.edits[key] = value
  }
  function toggleDelete(tab: TableTab, row: number) {
    const i = tab.deleted.indexOf(row)
    if (i >= 0) tab.deleted.splice(i, 1)
    else tab.deleted.push(row)
  }
  const discardEdits = (tab: TableTab) => { tab.edits = {}; tab.deleted = [] }

  /** The rows' changes as the commit wants them. A row whose key isn't shown (or was cut) can't be matched. */
  function gridEdits(tab: TableTab) {
    const rows: { key: { column: string, value: string }[], changes: { column: string, old: string | null, new: string | null }[], delete: boolean }[] = []
    const byRow = new Map<number, { column: string, old: string | null, new: string | null }[]>()
    for (const [k, v] of Object.entries(tab.edits)) {
      const [r, c] = k.split(':').map(Number) as [number, number]
      if (tab.deleted.includes(r)) continue
      byRow.set(r, [...(byRow.get(r) ?? []), { column: tab.cols[c]!.name, old: tab.rows[r]?.[c]?.v ?? null, new: v }])
    }
    for (const [r, changes] of byRow) rows.push({ key: keyOf(tab, r), changes, delete: false })
    for (const r of tab.deleted) rows.push({ key: keyOf(tab, r), changes: [], delete: true })
    return rows
  }

  /** "Review changes": Rust builds the statements and holds them; the dialog shows them. */
  async function review(tab: TableTab) {
    const rows = gridEdits(tab)
    if (rows.some(r => !r.key.length)) {
      tab.error = 'A row can\'t be matched without its whole key. Show the key columns, then try again.'
      return
    }
    try {
      tab.review = { pending: await api.sqlGridPrepare(connId.value, { schema: tab.schema, table: tab.table, rows }), error: '', busy: false }
    }
    catch (e) {
      tab.error = message(e, 'Couldn\'t prepare the changes.')
    }
  }
  async function commitReview(tab: TableTab) {
    const r = tab.review
    if (!r || r.busy) return
    r.busy = true
    r.error = ''
    try {
      await api.sqlGridCommit(connId.value, r.pending.id)
      tab.review = null
      discardEdits(tab)
      await loadPage(tab, tab.page)
    }
    catch (e) {
      r.error = message(e, 'Couldn\'t commit.')
      r.busy = false
    }
  }
  async function cancelReview(tab: TableTab) {
    const r = tab.review
    tab.review = null
    if (r) await api.sqlGridDiscard(connId.value, r.pending.id).catch(() => {})
  }

  // ---- Structure (S38, F24)
  async function setView(tab: TableTab, view: TableTab['view']) {
    tab.view = view
    if (view !== 'data' && !tab.structure) await loadStructure(tab)
  }
  async function loadStructure(tab: TableTab) {
    tab.structureError = ''
    try {
      tab.structure = await api.sqlStructure(connId.value, tab.schema, tab.table)
    }
    catch (e) {
      tab.structureError = message(e, 'Couldn\'t read this table\'s structure.')
    }
  }
  const addOp = (tab: TableTab, op: StructureOp) => { tab.ops.push(op) }
  const removeOp = (tab: TableTab, i: number) => { tab.ops.splice(i, 1) }
  /** The held statements, shown in the review dialog. */
  async function reviewStructure(tab: TableTab) {
    try {
      tab.structureReview = { pending: await api.sqlStructurePrepare(connId.value, tab.schema, tab.table, tab.ops), error: '', busy: false }
    }
    catch (e) {
      tab.structureError = message(e, 'Couldn\'t prepare the changes.')
    }
  }
  async function applyStructure(tab: TableTab, typed: string | null) {
    const r = tab.structureReview
    if (!r || r.busy) return
    r.busy = true
    r.error = ''
    try {
      await api.sqlStructureCommit(connId.value, r.pending.id, typed)
      tab.structureReview = null
      tab.ops = []
      // The tree, the columns and completion all changed.
      tab.structure = null
      tab.info = await api.sqlTableInfo(connId.value, tab.schema, tab.table).catch(() => tab.info)
      await loadStructure(tab)
      void loadCatalog(true)
      // What the old columns meant is gone: start the data again from its first page.
      Object.assign(tab, { edits: {}, deleted: [], filter: '', applied: '', sort: [], columns: [], cursors: [null], exact: null })
      await loadPage(tab, 0)
    }
    catch (e) {
      r.error = message(e, 'Couldn\'t apply the changes.')
      r.busy = false
    }
  }
  async function cancelStructure(tab: TableTab) {
    const r = tab.structureReview
    tab.structureReview = null
    if (r) await api.sqlStructureDiscard(connId.value, r.pending.id).catch(() => {})
  }
  /** "Copy as migration": the statements, without applying them (the gate still has to allow the change). */
  async function migrationText(tab: TableTab) {
    const p = await api.sqlStructurePrepare(connId.value, tab.schema, tab.table, tab.ops)
    await api.sqlStructureDiscard(connId.value, p.id).catch(() => {})
    return p.statements.map(x => `${x};`).join('\n')
  }

  // ---- Explain (S39, F25)
  async function explain(q: QueryTab, analyze: boolean, selected = '') {
    if (!connId.value) return
    const text = selected || q.text
    if (!text.trim()) return
    q.plan = { state: 'loading', analyze, plan: null, error: '', statement: '' }
    q.view = 'explain'
    try {
      const plan = await api.sqlExplain(connId.value, editorId(q), text, selected ? 'selection' : 'current', selected ? null : q.cursor, analyze, q.timeoutS * 1000)
      q.plan = { state: 'ready', analyze, plan, error: '', statement: '' }
    }
    catch (e) {
      q.plan = { state: 'failed', analyze, plan: null, error: message(e, 'Couldn\'t explain that.'), statement: '' }
    }
  }

  // ---- Activity (S41, F25) and Transfer (S43, F26)
  function openSpecial(kind: 'activity' | 'transfer') {
    if (!special.value.includes(kind)) special.value.push(kind)
    activeTab.value = kind
  }
  function closeSpecial(kind: 'activity' | 'transfer') {
    special.value = special.value.filter(k => k !== kind)
    if (activeTab.value === kind) activeTab.value = queries.value[0]?.id ?? tabs.value[0]?.id ?? ''
  }
  const activityState = reactive<{ view: ActivityView, data: Activity | null, error: string, at: number, rate: number | null, last: { total: number, at: number } | null, pending: SessionPending | null, busy: boolean, note: string }>(
    { view: 'sessions', data: null, error: '', at: 0, rate: null, last: null, pending: null, busy: false, note: '' })
  let activityToken = 0
  let activityBusy = false
  async function refreshActivity(view?: ActivityView) {
    if (!connId.value) return
    // A poll waits for the one before it; choosing another view starts afresh and drops the older answer.
    if (activityBusy && !view) return
    if (view && view !== activityState.view) { activityState.view = view; activityState.data = null }
    const token = ++activityToken
    activityBusy = true
    try {
      const d = await api.sqlActivity(connId.value, activityState.view)
      if (token !== activityToken) return
      const now = Date.now()
      // Transactions per second from two readings of the server's counter.
      const prev = activityState.last
      activityState.rate = prev && now > prev.at ? Math.max(0, ((d.tiles.transactions_total ?? 0) - prev.total) / ((now - prev.at) / 1000)) : activityState.rate
      activityState.last = { total: d.tiles.transactions_total ?? 0, at: now }
      activityState.data = d
      activityState.at = now
      activityState.error = ''
    }
    catch (e) {
      if (token === activityToken) activityState.error = message(e, 'Couldn\'t read the activity.')
    }
    finally {
      if (token === activityToken) activityBusy = false
    }
  }
  async function prepareSession(pid: number, action: SessionAction) {
    activityState.note = ''
    try {
      activityState.pending = await api.sqlSessionPrepare(connId.value, pid, action)
    }
    catch (e) {
      activityState.error = message(e, 'Couldn\'t prepare that.')
    }
  }
  async function confirmSession(typed: string | null) {
    const p = activityState.pending
    if (!p || activityState.busy) return
    activityState.busy = true
    try {
      const done = await api.sqlSessionCommit(connId.value, p.id, typed)
      activityState.pending = null
      activityState.note = done ? (p.end ? `Ended session ${p.pid}.` : `Cancelled the query of session ${p.pid}.`) : `Session ${p.pid} had already gone.`
      await refreshActivity()
    }
    catch (e) {
      activityState.error = message(e, 'Couldn\'t do that.')
    }
    activityState.busy = false
  }
  async function discardSession() {
    const p = activityState.pending
    activityState.pending = null
    if (p) await api.sqlSessionDiscard(connId.value, p.id).catch(() => {})
  }

  let transferCount = 0
  const newTransferId = () => `t${++transferCount}-${Date.now()}`
  const stopTransfer = (id: string) => api.sqlTransferStop(id).catch(() => {})
  const exportTable = (id: string, schema: string, table: string, filter: string, format: ExportFormat, header: boolean, onProgress: (p: TransferProgress) => void) => api.sqlExportTable(connId.value, id, schema, table, filter, format, header, onProgress)
  const importPick = () => api.sqlImportPick(connId.value)
  const importPreview = (schema: string, table: string, path: string, delimiter: string, header: boolean): Promise<CsvPreview> => api.sqlImportPreview(connId.value, schema, table, path, delimiter, header)
  const importPrepare = (schema: string, table: string, path: string, delimiter: string, header: boolean, mapping: (string | null)[]) => api.sqlImportPrepare(connId.value, schema, table, path, delimiter, header, mapping)
  const importCommit = (id: string) => api.sqlImportCommit(connId.value, id)
  const importDiscard = (id: string) => api.sqlImportDiscard(connId.value, id).catch(() => {})
  const backupCheck = () => api.sqlBackupCheck(connId.value)
  const backupStart = (id: string, onProgress: (p: TransferProgress) => void) => api.sqlBackupStart(connId.value, id, onProgress)
  const restorePrepare = () => api.sqlRestorePrepare(connId.value)
  const restoreCommit = (pending: string, typed: string | null, id: string, onProgress: (p: TransferProgress) => void) => api.sqlRestoreCommit(connId.value, pending, typed, id, onProgress)
  const restoreDiscard = (id: string) => api.sqlRestoreDiscard(connId.value, id).catch(() => {})

  // ---- Diagram (S40)
  async function openDiagram(schema?: string, table?: string) {
    const start = schema && table ? [`${schema}.${table}`] : []
    const n = ++diagramCount
    const d = reactive<DiagramTab>({ id: `d${n}`, title: table ? `Diagram · ${table}` : `Diagram ${n}`, start, steps: 1, keysOnly: false, data: null, loading: false, error: '' })
    diagrams.value.push(d)
    activeTab.value = d.id
    if (start.length) await loadDiagram(d)
  }
  async function loadDiagram(d: DiagramTab) {
    if (!d.start.length) return
    d.loading = true
    d.error = ''
    try {
      d.data = await api.sqlDiagram(connId.value, d.start.map((x) => { const [schema, ...t] = x.split('.'); return { schema: schema!, table: t.join('.') } }), d.steps)
    }
    catch (e) {
      d.error = message(e, 'Couldn\'t draw the diagram.')
    }
    d.loading = false
  }
  function closeDiagram(id: string) {
    const i = diagrams.value.findIndex(d => d.id === id)
    if (i < 0) return
    diagrams.value.splice(i, 1)
    if (activeTab.value === id) activeTab.value = diagrams.value[Math.min(i, diagrams.value.length - 1)]?.id ?? queries.value[0]?.id ?? tabs.value[0]?.id ?? ''
  }
  const saveExport = (name: string, bytes: Uint8Array) => api.sqlSaveExport(name, Array.from(bytes))

  /** Opens `schema.table` filtered by a foreign key's values: "Open row" and "Show them". */
  async function jump(schema: string, table: string, where: string) {
    await openTable(schema, { name: table, kind: 'table', estimate: null })
    const tab = tabs.value.find(t => t.id === tableId(schema, table))
    if (!tab) return
    tab.filter = where
    await apply(tab)
  }

  // ---- The query editor (S37, F23)
  const editorId = (q: QueryTab) => q.id

  function newQuery(text = '', title?: string, savedId: string | null = null) {
    const n = ++queryCount
    const q = reactive<QueryTab>({
      id: `q${n}`, title: title ?? `Query ${n}`, text, saved: savedId ? text : '', savedId, cursor: 0, running: false, results: [], messages: [], view: 'messages',
      limit: 1000, timeoutS: 30, totalMs: null, manual: false, txn: null, pending: null, plan: null, closeAsk: false,
    })
    queries.value.push(q)
    activeTab.value = q.id
    if (!catalog.value) void loadCatalog(false)
    return q
  }

  /** The × on a query tab: with a transaction open it asks first (S42). */
  function requestCloseQuery(q: QueryTab) {
    if (q.txn?.active) q.closeAsk = true
    else closeQuery(q.id)
  }
  /** Commit or roll back, then close. */
  async function finishAndClose(q: QueryTab, commit: boolean) {
    if (await txn(q, commit ? 'commit' : 'rollback')) {
      q.closeAsk = false
      closeQuery(q.id)
    }
  }
  /** Transactions open in this tab's editors (the tab itself closing asks about them). */
  const openTransactions = computed(() => queries.value.filter(q => q.txn?.active).length)

  function closeQuery(id: string) {
    const i = queries.value.findIndex(q => q.id === id)
    if (i < 0) return
    const q = queries.value[i]!
    if (q.running) void api.sqlCancel(connId.value, editorId(q)).catch(() => {})
    if (connId.value) void api.sqlEditorClose(connId.value, editorId(q)).catch(() => {})
    queries.value.splice(i, 1)
    if (activeTab.value === id) activeTab.value = queries.value[Math.min(i, queries.value.length - 1)]?.id ?? tabs.value[0]?.id ?? ''
  }

  async function loadCatalog(refresh: boolean) {
    if (!connId.value) return
    try {
      catalog.value = await api.sqlCatalog(connId.value, refresh)
    }
    catch {
      // Completion is a convenience: without the catalog the editor still offers keywords.
    }
  }

  const runIds = new WeakMap<QueryTab, number>()

  /**
   * Runs the statement at the cursor, the selection (`selected` is its text) or the whole script. Results arrive as
   * events; a newer run of the same tab drops what an older one still sends.
   */
  async function run(q: QueryTab, mode: RunMode, selected = '') {
    if (q.running || !connId.value) return
    const text = mode === 'selection' ? selected : q.text
    if (!text.trim()) {
      q.messages = [{ kind: 'info', text: mode === 'selection' ? 'Select some SQL first.' : 'There is nothing to run.' }]
      q.view = 'messages'
      return
    }
    // Manual: the transaction begins before the first statement.
    if (q.manual && !q.txn?.active && !(await txn(q, 'begin'))) return
    await start(q, events => api.sqlRun(connId.value, editorId(q), text, mode, mode === 'current' ? q.cursor : null, q.limit, q.timeoutS * 1000, events).then((r) => { q.pending = r.pending }))
  }

  /** The gate's yes: runs what it held. `typed` is the database's name for a dangerous change. */
  async function confirmRun(q: QueryTab, typed: string | null = null) {
    const p = q.pending
    if (!p || q.running) return
    q.pending = null
    await start(q, events => api.sqlRunCommit(connId.value, p.id, typed, q.limit, q.timeoutS * 1000, events))
    // A mistyped name leaves the confirmation valid: the dialog comes back to try again.
    if (q.messages.some(m => m.kind === 'error' && m.text.includes('doesn\'t match'))) q.pending = p
  }
  async function discardRun(q: QueryTab) {
    const p = q.pending
    q.pending = null
    if (p) await api.sqlRunDiscard(connId.value, p.id).catch(() => {})
  }

  /** Begin, commit or roll back the manual transaction; `state` just reads it. Returns false when the server said no. */
  async function txn(q: QueryTab, action: 'begin' | 'commit' | 'rollback' | 'state') {
    try {
      const st = await api.sqlTxn(connId.value, editorId(q), action)
      q.txn = st.active ? st : null
      return true
    }
    catch (e) {
      q.messages.push({ kind: 'error', text: message(e, 'Couldn\'t do that.') })
      q.view = 'messages'
      if (action === 'begin') q.manual = false
      return false
    }
  }
  async function setManual(q: QueryTab, on: boolean) {
    q.manual = on
    if (on) await txn(q, 'begin')
  }

  /** Streams one run's events into the tab. `call` makes the command; a newer run drops what an older one sends. */
  async function start(q: QueryTab, call: (onEvent: (e: SqlRunEvent_Serialize) => void) => Promise<unknown>) {
    const id = (runIds.get(q) ?? 0) + 1
    runIds.set(q, id)
    q.running = true
    q.results = []
    q.messages = []
    q.totalMs = null
    q.view = 'messages'
    const live = () => runIds.get(q) === id
    const onEvent = (e: SqlRunEvent_Serialize) => {
      if (!live()) return
      const r = q.results[('index' in e ? e.index : -1)]
      switch (e.type) {
        case 'started':
          q.results[e.index] = { index: e.index, statement: e.statement, cols: [], rows: [], state: 'running', more: false, elapsedMs: null, selected: null }
          break
        case 'columns':
          if (e.index === 0) q.view = 'r0'
          if (r) r.cols = e.columns.map(c => ({ name: c.name, type_name: c.type_name, kind: c.kind, nullable: true, default: null, pk_position: null }))
          break
        case 'rows':
          if (r) for (const row of e.rows) r.rows.push(row)
          break
        case 'done':
          if (r) {
            r.state = 'done'
            r.more = e.more
            r.elapsedMs = e.elapsed_ms
            q.messages.push({
              kind: 'info', statement: r.statement,
              text: e.affected !== null ? `${e.affected.toLocaleString('en-US')} ${e.affected === 1 ? 'row' : 'rows'} changed · ${e.elapsed_ms} ms`
                : r.cols.length ? `${e.rows.toLocaleString('en-US')} ${e.rows === 1 ? 'row' : 'rows'} · ${e.elapsed_ms} ms${e.more ? ` · stopped at the limit of ${q.limit.toLocaleString('en-US')}` : ''}` : `Done · ${e.elapsed_ms} ms`,
            })
          }
          break
        case 'failed':
          if (r) r.state = 'failed'
          q.messages.push({
            kind: 'error', statement: r?.statement,
            text: e.kind === 'timeout' ? `Stopped after ${q.timeoutS} s. Raise the timeout or add a filter.` : e.kind === 'cancelled' ? 'Stopped.' : e.message,
          })
          q.view = 'messages'
          break
        case 'refused':
          q.messages.push({ kind: 'refused', text: e.message })
          q.view = 'messages'
          break
        case 'finished':
          q.totalMs = e.elapsed_ms
          q.running = false
          void loadHistory()
          if (q.manual) void txn(q, 'state')
          break
      }
    }
    try {
      await call(onEvent)
    }
    catch (e) {
      if (live()) {
        q.messages.push({ kind: 'error', text: message(e, 'Couldn\'t run that.') })
        q.view = 'messages'
      }
    }
    // The command answers after its last event; if `finished` never came (a held change, or an error before the run began), stop here.
    if (live()) q.running = false
  }

  async function stop(q: QueryTab) {
    if (!q.running || !connId.value) return
    try {
      await api.sqlCancel(connId.value, editorId(q))
    }
    catch (e) {
      q.messages.push({ kind: 'error', text: message(e, 'Couldn\'t stop it.') })
    }
  }

  async function format(q: QueryTab) {
    if (q.text.trim()) q.text = await api.sqlFormat(q.text)
  }

  async function loadHistory() {
    if (!dbId.value) return
    try {
      history.value = await api.sqlHistory(dbId.value)
    }
    catch {
      history.value = []
    }
  }
  async function clearHistory() {
    await api.sqlHistoryClear(dbId.value)
    history.value = []
  }

  /** Save (⌘S): a new saved query, or the one this tab was loaded from. `thisDatabaseOnly` ties it to this database. */
  async function save(q: QueryTab, name: string, thisDatabaseOnly: boolean) {
    const row = await api.savedSave({ id: q.savedId, database_id: thisDatabaseOnly ? dbId.value : null, engine: 'postgres', kind: 'sql', name: name.trim(), folder: null, description: null, body: q.text })
    q.savedId = row.id
    q.saved = q.text
    q.title = row.name
  }

  return { dbId, connId, status, steps, prompt, info, error, schemas, treeFilter, tabs, activeTab, current, special, openSpecial, closeSpecial, activityState, refreshActivity, prepareSession, confirmSession, discardSession, newTransferId, stopTransfer, exportTable, importPick, importPreview, importPrepare, importCommit, importDiscard, backupCheck, backupStart, restorePrepare, restoreCommit, restoreDiscard, setView, loadStructure, addOp, removeOp, reviewStructure, applyStructure, cancelStructure, migrationText, explain, diagrams, openDiagram, loadDiagram, closeDiagram, saveExport, changeCount, setCell, toggleDelete, discardEdits, review, commitReview, cancelReview, queries, catalog, history, newQuery, closeQuery, requestCloseQuery, finishAndClose, openTransactions, confirmRun, discardRun, txn, setManual, loadCatalog, run, stop, format, loadHistory, clearHistory, save, open, retry, respond, close, toggleSchema, openTable, closeTable, loadPage, apply, refresh, nextPage, prevPage, sortBy, countExactly, select, keyOf, jump }
}

// One store per SQL tab, like the Redis tabs: each has its own connection, tree and open tables.
const makeStore = (key: string) => defineStore(`sql:${key}`, setup)
const tabStores = new Map<string, ReturnType<typeof makeStore>>()

export function useSqlStore(tabKey: string) {
  if (!tabStores.has(tabKey)) tabStores.set(tabKey, makeStore(tabKey))
  return tabStores.get(tabKey)!()
}

export function dropSqlStore(tabKey: string) {
  if (!tabStores.has(tabKey)) return
  useSqlStore(tabKey).close()
  tabStores.delete(tabKey)
}

/** Vault locked: Rust already closed every connection; forget the tabs' state. */
export function clearAllSql() {
  for (const key of [...tabStores.keys()]) dropSqlStore(key)
}
