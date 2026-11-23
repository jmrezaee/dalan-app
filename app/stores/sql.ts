import { defineStore } from 'pinia'
import type { SqlCatalog, SqlRunEvent_Serialize, RunMode, SqlCell_Serialize as Cell, DbEvent_Serialize, PageCursor_Serialize, PromptKind, RedisStepState, SqlColumn_Serialize as SqlColumn, SqlObject_Serialize as SqlObject, SqlServerInfo, SqlTableInfo_Serialize as SqlTableInfo } from '~/types/bindings'
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
}

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
export type QueryView = `r${number}` | 'messages' | 'history' | 'saved'
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
      keyset: false, loading: false, error: '', elapsedMs: null, estimate: o.estimate ?? null, exact: null, selected: null, full: null,
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
      limit: 1000, timeoutS: 30, totalMs: null,
    })
    queries.value.push(q)
    activeTab.value = q.id
    if (!catalog.value) void loadCatalog(false)
    return q
  }

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
          if (e.index === 0) q.view = 'r0'
          break
        case 'columns':
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
            q.messages.push({ kind: 'info', statement: r.statement, text: r.cols.length ? `${e.rows.toLocaleString('en-US')} ${e.rows === 1 ? 'row' : 'rows'} · ${e.elapsed_ms} ms${e.more ? ` · stopped at the limit of ${q.limit.toLocaleString('en-US')}` : ''}` : `Done · ${e.elapsed_ms} ms` })
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
          break
      }
    }
    try {
      await api.sqlRun(connId.value, editorId(q), text, mode, mode === 'current' ? q.cursor : null, q.limit, q.timeoutS * 1000, onEvent)
    }
    catch (e) {
      if (live()) {
        q.messages.push({ kind: 'error', text: message(e, 'Couldn\'t run that.') })
        q.view = 'messages'
      }
    }
    // The command answers after its last event; if `finished` never came (an error before the run began), stop here.
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

  return { dbId, connId, status, steps, prompt, info, error, schemas, treeFilter, tabs, activeTab, current, queries, catalog, history, newQuery, closeQuery, loadCatalog, run, stop, format, loadHistory, clearHistory, save, open, retry, respond, close, toggleSchema, openTable, closeTable, loadPage, apply, refresh, nextPage, prevPage, sortBy, countExactly, select, keyOf, jump }
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
