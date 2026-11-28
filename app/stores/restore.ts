import { defineStore } from 'pinia'
import type { RestoreOptions_Serialize as SavedRestore, DumpInfo_Serialize as DumpInfo, RestoreOptions_Deserialize as RestoreOptions, RestorePlan_Serialize as RestorePlan, RestoreProgress_Serialize as RestoreProgress, RestoreResult_Serialize as RestoreResult } from '~/types/bindings'
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
  /** The saved job being run: its dump is read again from where it was saved, and the run is recorded. */
  const jobId = ref<string | null>(null)
  const saved = ref('')
  /** The tables or collections chosen in "What's inside"; `null` is all of them. */
  const selected = ref<string[] | null>(null)
  const options = reactive<Omit<RestoreOptions, 'new_database' | 'objects' | 'rename_schema'> & { target: 'this' | 'new', newName: string, renameFrom: string, renameTo: string }>({
    target: 'this',
    newName: '',
    renameFrom: '',
    renameTo: '',
    jobs: 1,
    disable_triggers: false,
    source_database: null,
    existing: 'stop',
    content: 'all',
    skip_owner_acl: true,
    include_roles: false,
    single_transaction: true,
  })

  function start(id: string, production: boolean) {
    connId.value = id
    jobId.value = null
    saved.value = ''
    selected.value = null
    Object.assign(options, { target: production ? 'new' : 'this', newName: '', renameFrom: '', renameTo: '', jobs: 1, disable_triggers: false, source_database: null, existing: 'stop', content: 'all', skip_owner_acl: true, include_roles: false, single_transaction: true })
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

  /** D15 (E8): a dump in a folder on a host, picked in the SFTP picker; Rust fetches it and reads it. */
  async function pickRemote(sftpId: string, hostId: string, path: string) {
    error.value = ''
    busy.value = true
    try {
      const d = await api.restorePickRemote(connId.value, sftpId, hostId, path)
      if (dump.value) void api.restoreForget(connId.value, dump.value.id).catch(() => {})
      dump.value = d
      options.source_database = d.databases.length > 1 ? (d.databases.find(x => x === d.source_db) ?? d.databases[0] ?? null) : null
      step.value = 'inside'
    }
    catch (e) { error.value = message(e, 'Couldn\'t fetch that file.') }
    busy.value = false
  }

  async function review() {
    const d = dump.value
    if (!d) return
    error.value = ''
    busy.value = true
    try {
      const { target, newName, renameFrom, renameTo, ...rest } = options
      plan.value = await api.restorePrepare(connId.value, d.id, {
        ...rest,
        new_database: target === 'new' ? newName.trim() : null,
        objects: selected.value,
        rename_schema: renameFrom && renameTo.trim() ? { from: renameFrom, to: renameTo.trim() } : null,
      })
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
      result.value = await api.restoreCommit(connId.value, p.pending_id, p.level === 'typed' ? typed.value : null, transferId.value, (x) => { progress.value = x }, jobId.value)
      step.value = 'result'
      if (jobId.value) void useJobsStore().refresh()
    }
    catch (e) {
      // Refused by the gate or failed before the tool ran: back to the review, which says why.
      error.value = message(e, 'The restore didn\'t start.')
      step.value = 'review'
      plan.value = { ...p, pending_id: null }
    }
  }

  function optionsForRequest() {
    const { target, newName, renameFrom, renameTo, ...rest } = options
    return { ...rest, new_database: target === 'new' ? newName.trim() : null, objects: selected.value, rename_schema: renameFrom && renameTo.trim() ? { from: renameFrom, to: renameTo.trim() } : null }
  }

  /** Runs a saved restore: the dump is read again (and checked again), the saved options are set, and the review follows. */
  async function startJob(conn: string, id: string, saved: SavedRestore | null, production: boolean) {
    start(conn, production)
    jobId.value = id
    busy.value = true
    try {
      const d = await api.restorePickJob(conn, id)
      dump.value = d
      if (saved) {
        Object.assign(options, { target: saved.new_database ? 'new' : 'this', newName: saved.new_database ?? '', source_database: saved.source_database ?? null, existing: saved.existing, content: saved.content, skip_owner_acl: saved.skip_owner_acl, include_roles: saved.include_roles, single_transaction: saved.single_transaction, jobs: saved.jobs ?? 1, disable_triggers: saved.disable_triggers ?? false, renameFrom: saved.rename_schema?.from ?? '', renameTo: saved.rename_schema?.to ?? '' })
        selected.value = saved.objects ?? null
      }
      step.value = 'inside'
    }
    catch (e) { error.value = message(e, 'Couldn\'t read the job\'s dump.') }
    busy.value = false
  }

  /** Saves this restore (the dump stays where it is; Rust keeps its path) as a job. */
  async function saveJob(name: string) {
    const d = dump.value
    if (!d) return
    error.value = ''
    try {
      const row = await api.jobSaveRestore(connId.value, d.id, name, optionsForRequest())
      // This restore, if it runs now, is the job's first run.
      jobId.value = row.id
      saved.value = `Saved as the job “${row.name}”. Run it again from Jobs.`
      void useJobsStore().refresh()
    }
    catch (e) { error.value = message(e, 'Couldn\'t save the job.') }
  }

  const stop = () => api.sqlTransferStop(transferId.value).catch(() => {})

  function close() {
    if (step.value === 'progress') return
    open.value = false
    if (dump.value) void api.restoreForget(connId.value, dump.value.id).catch(() => {})
    dump.value = null
  }

  /** After a restore into a new database: save it as a database (a copy of this one's connection) to open it. */
  const saveDatabase = (name: string) => api.restoreSaveDatabase(connId.value, name)

  return { jobId, saved, startJob, saveJob, connId, open, step, busy, error, dump, plan, progress, result, typed, options, selected, startedAt, start, pick, pickRemote, review, commit, stop, close, saveDatabase }
}

const makeStore = (key: string) => defineStore(`restore:${key}`, setup)
const stores = new Map<string, ReturnType<typeof makeStore>>()

/** One restore dialog per database tab. */
export function useRestoreStore(tabKey: string) {
  if (!stores.has(tabKey)) stores.set(tabKey, makeStore(tabKey))
  return stores.get(tabKey)!()
}

/** Databases whose tab should open its restore dialog once connected ("Restore a dump…" in the database list). */
export const restoreRequests = ref(new Set<string>())
