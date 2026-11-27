import { defineStore } from 'pinia'
import type { DataColumns, DataCompareResult_Serialize as DataResult, DataProgress, DataSide, MatchPlan, MatchReport, SchemaCompare_Serialize as SchemaCompare, Side_Deserialize as Side, SnapshotRow, SyncPlan, SyncReport_Serialize as SyncReport } from '~/types/bindings'
import { useTauri } from '~/composables/useTauri'

const message = (e: unknown, fallback: string) => (e instanceof Error ? e.message : (e as { message?: string })?.message ?? fallback)

/** Where one side comes from: a schema of an open tab (by tab key), a saved snapshot, or a dump read on a tab's server. */
export interface SideChoice { kind: 'database' | 'snapshot' | 'dump', tabKey: string, schema: string, snapshotId: string, dumpId: string, dumpName: string }

/** One table of a data compare: an open tab, a schema (MySQL: database) and a table. */
export interface TableChoice { tabKey: string, schema: string, table: string }

/**
 * Schema compare and sync (D16, E3) for one database tab. Rust reads both sides fresh on every call (compare, the
 * script, "Sync"), so what is held for the gate is never older than the confirmation.
 */
function setup() {
  const api = useTauri()
  const mode = ref<'schema' | 'data'>('schema')
  const source = reactive<SideChoice>({ kind: 'database', tabKey: '', schema: '', snapshotId: '', dumpId: '', dumpName: '' })
  const target = reactive<SideChoice>({ kind: 'database', tabKey: '', schema: '', snapshotId: '', dumpId: '', dumpName: '' })
  const result = ref<SchemaCompare | null>(null)
  const chosen = ref(new Set<string>())
  const selected = ref<string | null>(null)
  const snapshots = ref<SnapshotRow[]>([])
  const plan = ref<SyncPlan | null>(null)
  const report = ref<SyncReport | null>(null)
  const typed = ref('')
  const busy = ref(false)
  const error = ref('')
  const note = ref('')

  function side(c: SideChoice): Side {
    if (c.kind === 'snapshot') return { kind: 'snapshot', id: c.snapshotId }
    if (c.kind === 'dump') return { kind: 'dump', conn_id: useSqlStore(c.tabKey).connId, dump_id: c.dumpId, schema: c.schema || null }
    const st = useSqlStore(c.tabKey)
    return { kind: 'database', conn_id: st.connId, schema: c.schema }
  }

  async function run<T>(fn: () => Promise<T>, fallback: string): Promise<T | undefined> {
    busy.value = true
    error.value = ''
    try { return await fn() }
    catch (e) { error.value = message(e, fallback) }
    finally { busy.value = false }
  }

  async function loadSnapshots() {
    try { snapshots.value = await api.schemaSnapshots() }
    catch { snapshots.value = [] }
  }

  async function compare() {
    plan.value = null
    report.value = null
    note.value = ''
    const r = await run(() => api.schemaCompare(side(source), side(target)), 'Couldn\'t compare them.')
    if (r) {
      result.value = r
      // Everything is ticked except drops, which are opted into.
      chosen.value = new Set(r.items.filter(i => i.change !== 'removed').map(i => i.id))
      selected.value = r.items[0]?.id ?? null
    }
  }

  async function snapshot(c: SideChoice) {
    const st = useSqlStore(c.tabKey)
    const row = await run(() => api.schemaSnapshotSave(st.connId, c.schema), 'Couldn\'t save the snapshot.')
    if (row) {
      note.value = `Saved a snapshot of ${row.database_name} · ${row.schema}.`
      await loadSnapshots()
    }
  }

  async function removeSnapshot(id: string) {
    await run(() => api.schemaSnapshotDelete(id), 'Couldn\'t delete the snapshot.')
    await loadSnapshots()
  }

  const ids = () => [...chosen.value]
  async function script() {
    return run(() => api.schemaSyncScript(side(source), side(target), ids()), 'Couldn\'t build the script.')
  }
  async function saveScript() {
    const name = await run(() => api.schemaScriptSave(side(source), side(target), ids()), 'Couldn\'t save the script.')
    if (name) note.value = `Saved ${name}.`
  }
  async function prepare() {
    report.value = null
    typed.value = ''
    plan.value = (await run(() => api.schemaSyncPrepare(side(source), side(target), ids()), 'Couldn\'t prepare the sync.')) ?? null
  }
  async function commit() {
    const p = plan.value
    if (!p) return
    const connId = useSqlStore(target.tabKey).connId
    const r = await run(() => api.schemaSyncCommit(connId, p.pending_id, p.level === 'typed' ? typed.value : null), 'The sync didn\'t run.')
    if (r) {
      report.value = r
      plan.value = null
    }
  }
  function cancelPlan() {
    const p = plan.value
    if (p) void api.schemaSyncDiscard(useSqlStore(target.tabKey).connId, p.pending_id).catch(() => {})
    plan.value = null
  }

  /** A dump file for a side: Rust opens the dialog and keeps the file with that tab (the window holds its id). */
  async function pickDump(c: SideChoice) {
    const d = await run(() => api.restorePick(useSqlStore(c.tabKey).connId, false), 'Couldn\'t read that file.')
    if (d) {
      c.dumpId = d.id
      c.dumpName = d.file_name
    }
  }

  // ---- Data compare (E4)
  const dsrc = reactive<TableChoice>({ tabKey: '', schema: '', table: '' })
  const dtgt = reactive<TableChoice>({ tabKey: '', schema: '', table: '' })
  const columns = ref<DataColumns | null>(null)
  const key = ref<string[]>([])
  const quick = ref(true)
  const data = ref<DataResult | null>(null)
  const dprogress = ref<DataProgress | null>(null)
  const match = ref<MatchPlan | null>(null)
  const matched = ref<MatchReport | null>(null)
  const transferId = ref('')
  const table = (t: TableChoice): DataSide => ({ conn_id: useSqlStore(t.tabKey).connId, schema: t.schema, table: t.table.trim() })
  async function readColumns() {
    data.value = null
    match.value = null
    matched.value = null
    const c = await run(() => api.dataColumns(table(dsrc), table(dtgt)), 'Couldn\'t read the tables.')
    columns.value = c ?? null
    if (c) key.value = [...c.primary_key]
  }
  async function compareData() {
    matched.value = null
    match.value = null
    transferId.value = crypto.randomUUID()
    dprogress.value = null
    data.value = (await run(() => api.dataCompare(table(dsrc), table(dtgt), key.value, quick.value, transferId.value, (p) => { dprogress.value = p }), 'The compare didn\'t finish.')) ?? null
    transferId.value = ''
  }
  function stopData() {
    if (transferId.value) void api.sqlTransferStop(transferId.value).catch(() => {})
  }
  async function prepareMatch() {
    typed.value = ''
    match.value = (await run(() => api.dataMatchPrepare(table(dsrc), table(dtgt), key.value, quick.value), 'Couldn\'t prepare it.')) ?? null
  }
  async function commitMatch() {
    const p = match.value
    if (!p) return
    transferId.value = crypto.randomUUID()
    const r = await run(() => api.dataMatchCommit(table(dtgt).conn_id, p.pending_id, p.level === 'typed' ? typed.value : null, transferId.value, (x) => { dprogress.value = x }), 'Nothing changed: it didn\'t finish.')
    transferId.value = ''
    if (r) {
      matched.value = r
      match.value = null
    }
  }
  function cancelMatch() {
    const p = match.value
    if (p) void api.dataMatchDiscard(table(dtgt).conn_id, p.pending_id).catch(() => {})
    match.value = null
  }
  async function saveDataScript() {
    const name = await run(() => api.dataMatchScript(table(dsrc), table(dtgt), key.value, quick.value), 'Couldn\'t save the script.')
    if (name) note.value = `Saved ${name}.`
  }

  return { mode, pickDump, dsrc, dtgt, columns, key, quick, data, dprogress, match, matched, transferId, readColumns, compareData, stopData, prepareMatch, commitMatch, cancelMatch, saveDataScript, source, target, result, chosen, selected, snapshots, plan, report, typed, busy, error, note, loadSnapshots, compare, snapshot, removeSnapshot, script, saveScript, prepare, commit, cancelPlan }
}

const makeStore = (key: string) => defineStore(`compare:${key}`, setup)
const stores = new Map<string, ReturnType<typeof makeStore>>()

/** One Compare tab per database tab. */
export function useCompareStore(tabKey: string) {
  if (!stores.has(tabKey)) stores.set(tabKey, makeStore(tabKey))
  return stores.get(tabKey)!()
}
