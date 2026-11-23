import { defineStore } from 'pinia'
import type { DbEvent_Serialize, MgCount, MgDoc, MgDocOp, MgNode, MgPending_Serialize as MgPending, PromptKind, RedisStepState, SqlServerInfo } from '~/types/bindings'
import { useTauri } from '~/composables/useTauri'

// MongoDB tabs (S44, F27): the connection like a SQL tab's, a tree of databases and collections, and one inner tab per
// open collection with the query bar, the three views and the document editor. Writes go through the gate: the window
// asks Rust to prepare a change, shows what it will do, and commits it once.

export type MongoStatus = 'connecting' | 'ready' | 'failed' | 'closed'
export interface MongoStep { label: string, note: string | null, detail: string | null, state: 'pending' | RedisStepState }
export interface DbNode { name: string, open: boolean, loading: boolean, colls: MgNode[] | null, error: string }
export interface Editor { mode: 'edit' | 'new', id: string | null, text: string, /** What the editor opened with, to tell unsaved typing from none. */ original: string, error: string }
export interface Review { pending: MgPending, error: string, busy: boolean }
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
  /** Which request is the latest: an older answer must not overwrite a newer page. */
  seq: number
  busy: boolean
}

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
    const t: CollTab = { id, db, coll: c.name, kind: c.kind, ...blank, applied: { ...blank }, limit: 50, page: 0, view: 'list', docs: [], more: false, loading: false, error: '', tookMs: null, count: null, selected: null, editor: null, review: null, seq: 0, busy: false }
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
    if (t.busy || t.review) return false
    t.busy = true
    try {
      t.review = { pending: await api.mgDocPrepare(connId.value, t.db, t.coll, op), error: '', busy: false }
      return true
    }
    catch (e) {
      const msg = message(e, 'Couldn\'t prepare that change.')
      if (t.editor) t.editor.error = msg
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
      await api.mgDocCommit(connId.value, r.pending.id)
      t.review = null
      t.editor = null
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
      t.review = null
      if (t.editor) t.editor.error = r.error
      else t.error = r.error
    }
  }
  function cancelReview(t: CollTab) {
    const r = t.review
    t.review = null
    if (r) void api.mgDocDiscard(connId.value, r.pending.id)
  }

  return { dbId, connId, status, steps, prompt, info, error, dbs, treeFilter, tabs, activeTab, current, open, retry, respond, close, toggleDb, openColl, closeColl, load, apply, setLimit, nextPage, prevPage, countDocs, startEdit, startNew, saveEditor, deleteDoc, commitReview, cancelReview }
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
