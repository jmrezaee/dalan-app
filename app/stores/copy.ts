import { defineStore } from 'pinia'
import type { CopyContent, CopyMode, CopyObject_Deserialize as CopyObject, CopyPlan_Serialize as CopyPlan, CopyProgress_Serialize as CopyProgress, CopyReport_Serialize as CopyReport } from '~/types/bindings'
import { useTauri } from '~/composables/useTauri'

/** S49's steps: choose (source, target, objects, options), review, progress, report. */
export type CopyStep = 'choose' | 'review' | 'progress' | 'report'

/** One object of the source as the Copy tab shows it, with what the person set for it. */
export interface CopyPick { schema: string, name: string, kind: string, checked: boolean, rename: string, filter: string, limit: string, /** Between engines: target types chosen in the review, by column. */ types: Record<string, string>, /** Between engines: the edited DDL (`null`: the generated one). */ ddl: string | null }

const message = (e: unknown, fallback: string) => (e instanceof Error ? e.message : (e as { message?: string })?.message ?? fallback)
export const pickKey = (p: { schema: string, name: string }) => `${p.schema}\u0000${p.name}`

/**
 * Copy between databases (S49, F32) for one source tab. Rust plans, holds the copy through the target's gate and
 * runs it; this keeps what the Copy tab shows.
 */
function setup() {
  const api = useTauri()
  const step = ref<CopyStep>('choose')
  const targetKey = ref('')
  const targetNew = ref(false)
  const newName = ref('')
  const picks = ref<Record<string, CopyPick>>({})
  const mode = ref<CopyMode>('create')
  const content = ref<CopyContent>('all')
  const stopOnError = ref(true)
  const routines = ref(false)
  const lastTarget = ref('')
  const busy = ref(false)
  const error = ref('')
  const plan = ref<CopyPlan | null>(null)
  const typed = ref('')
  const progress = ref<CopyProgress | null>(null)
  const report = ref<CopyReport | null>(null)
  const transferId = ref('')
  const startedAt = ref(0)

  /** Adds the source's objects to the list (keeping what was set for ones already there). */
  function offer(objects: { schema: string, name: string, kind: string }[]) {
    for (const o of objects) {
      const k = pickKey(o)
      if (!picks.value[k]) picks.value[k] = { ...o, checked: false, rename: '', filter: '', limit: '', types: {}, ddl: null }
    }
  }

  const chosen = computed(() => Object.values(picks.value).filter(p => p.checked))

  function request(targetConnId: string): { target_conn_id: string, new_database: string | null, objects: CopyObject[], mode: CopyMode, content: CopyContent, stop_on_error: boolean, routines: boolean } {
    return {
      target_conn_id: targetConnId,
      new_database: targetNew.value ? newName.value.trim() : null,
      objects: chosen.value.map(p => ({
        schema: p.schema,
        name: p.name,
        kind: p.kind === 'matview' ? 'matview' : p.kind,
        filter: p.filter.trim() || null,
        limit: p.limit.trim() === '' ? null : Number(p.limit),
        rename: p.rename.trim() || null,
        types: Object.keys(p.types).length ? Object.entries(p.types).map(([name, target_type]) => ({ name, target_type })) : null,
        ddl: p.ddl?.trim() || null,
      })),
      mode: mode.value,
      content: content.value,
      stop_on_error: stopOnError.value,
      routines: routines.value,
    }
  }

  async function review(sourceConnId: string, targetConnId: string) {
    lastTarget.value = targetConnId
    error.value = ''
    busy.value = true
    try {
      plan.value = await api.copyPrepare(sourceConnId, request(targetConnId))
      typed.value = ''
      step.value = 'review'
    }
    catch (e) { error.value = message(e, 'Couldn\'t plan the copy.') }
    busy.value = false
  }

  async function commit(sourceConnId: string) {
    const p = plan.value
    if (!p?.pending_id) return
    error.value = ''
    transferId.value = `copy-${Date.now()}`
    startedAt.value = Date.now()
    progress.value = null
    step.value = 'progress'
    try {
      report.value = await api.copyCommit(sourceConnId, p.pending_id, p.level === 'typed' ? typed.value : null, transferId.value, (x) => { progress.value = x })
      step.value = 'report'
    }
    catch (e) {
      error.value = message(e, 'The copy didn\'t start.')
      step.value = 'review'
      plan.value = { ...p, pending_id: null }
    }
  }

  /** "Select related": ticks the tables the chosen ones reference by foreign key. */
  async function selectRelated(sourceConnId: string) {
    error.value = ''
    try {
      const more = await api.copyRelated(sourceConnId, request('').objects)
      offer(more)
      for (const o of more) picks.value[pickKey(o)]!.checked = true
    }
    catch (e) { error.value = message(e, 'Couldn\'t follow the foreign keys.') }
  }

  const stop = () => api.sqlTransferStop(transferId.value).catch(() => {})

  /** "Run again from the failed table": that table and the ones after it, the same options. */
  function again() {
    const r = report.value
    if (!r) return
    const from = r.tables.findIndex(t => t.status === 'failed' || t.status === 'stopped')
    const left = new Set(r.tables.slice(Math.max(0, from)).filter(t => t.status !== 'done').map(t => t.source))
    for (const p of Object.values(picks.value)) {
      const shown = p.kind === 'collection' ? p.name : `${p.schema}.${p.name}`
      p.checked = p.checked && left.has(shown)
    }
    step.value = 'choose'
    report.value = null
  }

  function reset() {
    step.value = 'choose'
    plan.value = null
    report.value = null
    error.value = ''
  }

  return { routines, lastTarget, selectRelated, step, targetKey, targetNew, newName, picks, mode, content, stopOnError, busy, error, plan, typed, progress, report, startedAt, chosen, offer, review, commit, stop, again, reset }
}

const makeStore = (key: string) => defineStore(`copy:${key}`, setup)
const stores = new Map<string, ReturnType<typeof makeStore>>()

/** One Copy tab per database tab. */
export function useCopyStore(tabKey: string) {
  if (!stores.has(tabKey)) stores.set(tabKey, makeStore(tabKey))
  return stores.get(tabKey)!()
}

/** Databases whose tab should open its Copy tab once connected ("Copy to…" in the database list). */
export const copyRequests = ref(new Set<string>())
