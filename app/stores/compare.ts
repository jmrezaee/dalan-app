import { defineStore } from 'pinia'
import type { SchemaCompare_Serialize as SchemaCompare, Side, SnapshotRow, SyncPlan, SyncReport_Serialize as SyncReport } from '~/types/bindings'
import { useTauri } from '~/composables/useTauri'

const message = (e: unknown, fallback: string) => (e instanceof Error ? e.message : (e as { message?: string })?.message ?? fallback)

/** Where one side comes from: a schema of an open tab (by tab key), or a saved snapshot. */
export interface SideChoice { kind: 'database' | 'snapshot', tabKey: string, schema: string, snapshotId: string }

/**
 * Schema compare and sync (D16, E3) for one database tab. Rust reads both sides fresh on every call (compare, the
 * script, "Sync"), so what is held for the gate is never older than the confirmation.
 */
function setup() {
  const api = useTauri()
  const source = reactive<SideChoice>({ kind: 'database', tabKey: '', schema: '', snapshotId: '' })
  const target = reactive<SideChoice>({ kind: 'database', tabKey: '', schema: '', snapshotId: '' })
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

  return { source, target, result, chosen, selected, snapshots, plan, report, typed, busy, error, note, loadSnapshots, compare, snapshot, removeSnapshot, script, saveScript, prepare, commit, cancelPlan }
}

const makeStore = (key: string) => defineStore(`compare:${key}`, setup)
const stores = new Map<string, ReturnType<typeof makeStore>>()

/** One Compare tab per database tab. */
export function useCompareStore(tabKey: string) {
  if (!stores.has(tabKey)) stores.set(tabKey, makeStore(tabKey))
  return stores.get(tabKey)!()
}
