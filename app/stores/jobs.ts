import { defineStore } from 'pinia'
import type { CopyRequest_Serialize as CopyRequest, JobRow_Serialize as JobRow, JobRun_Serialize as JobRun, RestoreOptions_Serialize as RestoreOptions } from '~/types/bindings'
import { useTauri } from '~/composables/useTauri'

/** What a database tab does once connected, for a saved job being run (D14, E1). */
export interface JobRequest { jobId: string, kind: 'copy' | 'restore' | 'backup', copy: CopyRequest | null, restore: RestoreOptions | null, targetDbId: string | null }

/** Saved backups for a tab's Transfer view to run, by tab key. */
export const backupRequests = ref(new Map<string, string>())

/** Saved jobs waiting for their database tab to connect, by database id. */
export const jobRequests = ref(new Map<string, JobRequest>())

const message = (e: unknown, fallback: string) => (e instanceof Error ? e.message : (e as { message?: string })?.message ?? fallback)

/**
 * Saved jobs (D14, E1): the list, each job's runs, and running one. A run opens the database's tab and goes through
 * the same review as the first time (Rust builds the confirmation fresh): nothing approved earlier carries over.
 */
export const useJobsStore = defineStore('jobs', () => {
  const api = useTauri()
  const rows = ref<JobRow[]>([])
  const error = ref('')
  const runs = ref<Record<string, JobRun[]>>({})

  async function refresh() {
    try { rows.value = await api.jobsList(); error.value = '' }
    catch (e) { error.value = message(e, 'Couldn\'t list the jobs.') }
  }

  async function loadRuns(id: string) {
    runs.value[id] = await api.jobRuns(id)
  }

  async function rename(id: string, name: string) {
    await api.jobRename(id, name)
    await refresh()
  }

  async function remove(id: string) {
    await api.jobDelete(id)
    await refresh()
  }

  /** Opens the job's database tabs; each tab takes its part once connected (see `jobRequests`). */
  async function run(job: JobRow) {
    error.value = ''
    try {
      const setup = await api.jobSetup(job.id)
      const dbs = useDatabasesStore()
      const sessions = useSessionsStore()
      const open = (id: string | null) => {
        const d = dbs.rows.find(x => x.id === id)
        if (!d) throw new Error('A database this job uses is no longer saved.')
        if (d.engine === 'mongodb') sessions.openMongo(d)
        else sessions.openSql(d)
      }
      if (setup.kind === 'copy') {
        // The target first: the copy needs it open as a tab.
        open(setup.target_database_id)
        jobRequests.value.set(setup.source_database_id!, { jobId: job.id, kind: 'copy', copy: setup.copy, restore: null, targetDbId: setup.target_database_id })
        open(setup.source_database_id)
      }
      else if (setup.kind === 'restore') {
        jobRequests.value.set(setup.target_database_id!, { jobId: job.id, kind: 'restore', copy: null, restore: setup.restore, targetDbId: null })
        open(setup.target_database_id)
      }
      else {
        jobRequests.value.set(setup.source_database_id!, { jobId: job.id, kind: 'backup', copy: null, restore: null, targetDbId: null })
        open(setup.source_database_id)
      }
    }
    catch (e) { error.value = message(e, 'Couldn\'t start the job.') }
  }

  return { rows, error, runs, refresh, loadRuns, rename, remove, run }
})
