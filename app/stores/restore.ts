import { defineStore } from 'pinia'
import type { DumpInfo_Serialize as DumpInfo, RestoreOptions_Deserialize as RestoreOptions, RestorePlan_Serialize as RestorePlan, RestoreProgress_Serialize as RestoreProgress, RestoreResult_Serialize as RestoreResult } from '~/types/bindings'
import { useTauri } from '~/composables/useTauri'

/** S48's steps, in order. */
export type RestoreStep = 'file' | 'inside' | 'target' | 'review' | 'progress' | 'result'

const message = (e: unknown, fallback: string) => (e instanceof Error ? e.message : (e as { message?: string })?.message ?? fallback)

/**
 * Restore a dump (S48, F31) for one database tab: pick, what's inside, target and options, review, progress,
 * result. Rust reads the file and decides; this holds what the dialog shows.
 */
function setup() {
  const api = useTauri()
  const connId = ref('')
  const open = ref(false)
  const step = ref<RestoreStep>('file')
  const busy = ref(false)
  const error = ref('')
  const dump = ref<DumpInfo | null>(null)
  const plan = ref<RestorePlan | null>(null)
  const progress = ref<RestoreProgress | null>(null)
  const result = ref<RestoreResult | null>(null)
  const typed = ref('')
  const transferId = ref('')
  const startedAt = ref(0)
  const options = reactive<Omit<RestoreOptions, 'new_database'> & { target: 'this' | 'new', newName: string }>({
    target: 'this',
    newName: '',
    source_database: null,
    existing: 'stop',
    content: 'all',
    skip_owner_acl: true,
    include_roles: false,
    single_transaction: true,
  })

  function start(id: string, production: boolean) {
    connId.value = id
    Object.assign(options, { target: production ? 'new' : 'this', newName: '', source_database: null, existing: 'stop', content: 'all', skip_owner_acl: true, include_roles: false, single_transaction: true })
    step.value = 'file'
    error.value = ''
    dump.value = null
    plan.value = null
    result.value = null
    progress.value = null
    typed.value = ''
    open.value = true
  }

  async function pick(folder: boolean) {
    error.value = ''
    busy.value = true
    try {
      const d = await api.restorePick(connId.value, folder)
      if (d) {
        if (dump.value) void api.restoreForget(connId.value, dump.value.id).catch(() => {})
        dump.value = d
        options.source_database = d.databases.length > 1 ? (d.databases.find(x => x === d.source_db) ?? d.databases[0] ?? null) : null
        step.value = 'inside'
      }
    }
    catch (e) { error.value = message(e, 'Couldn\'t read that file.') }
    busy.value = false
  }

  async function review() {
    const d = dump.value
    if (!d) return
    error.value = ''
    busy.value = true
    try {
      const { target, newName, ...rest } = options
      plan.value = await api.restorePrepare(connId.value, d.id, { ...rest, new_database: target === 'new' ? newName.trim() : null })
      typed.value = ''
      step.value = 'review'
    }
    catch (e) { error.value = message(e, 'Couldn\'t plan the restore.') }
    busy.value = false
  }

  async function commit() {
    const p = plan.value
    if (!p?.pending_id) return
    error.value = ''
    transferId.value = `restore-${Date.now()}`
    startedAt.value = Date.now()
    progress.value = null
    step.value = 'progress'
    try {
      result.value = await api.restoreCommit(connId.value, p.pending_id, p.level === 'typed' ? typed.value : null, transferId.value, (x) => { progress.value = x })
      step.value = 'result'
    }
    catch (e) {
      // Refused by the gate or failed before the tool ran: back to the review, which says why.
      error.value = message(e, 'The restore didn\'t start.')
      step.value = 'review'
      plan.value = { ...p, pending_id: null }
    }
  }

  const stop = () => api.sqlTransferStop(transferId.value).catch(() => {})

  function close() {
    if (step.value === 'progress') return
    open.value = false
    if (dump.value) void api.restoreForget(connId.value, dump.value.id).catch(() => {})
    dump.value = null
  }

  return { connId, open, step, busy, error, dump, plan, progress, result, typed, options, startedAt, start, pick, review, commit, stop, close }
}

const makeStore = (key: string) => defineStore(`restore:${key}`, setup)
const stores = new Map<string, ReturnType<typeof makeStore>>()

/** One restore dialog per database tab. */
export function useRestoreStore(tabKey: string) {
  if (!stores.has(tabKey)) stores.set(tabKey, makeStore(tabKey))
  return stores.get(tabKey)!()
}
