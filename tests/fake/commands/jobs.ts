// Saved jobs (D14, E1). The fake keeps jobs and runs in memory; a run is recorded when a copy, restore or backup
// that carries the job's id ends. Nothing here holds a password, like the real thing.
import { fail, type Handlers } from '../types'
import { dumpFor } from './restore'
import type { JobRow_Serialize, JobRun_Serialize, CopyRequest_Deserialize, RestoreOptions_Deserialize } from '../../../app/types/bindings'

type Keys = 'jobsList' | 'jobRuns' | 'jobRename' | 'jobDelete' | 'jobSaveCopy' | 'jobSaveRestore' | 'jobSaveBackup' | 'jobSetup' | 'restorePickJob' | 'jobBackupRun'

interface FakeJob { row: JobRow_Serialize, copy: CopyRequest_Deserialize | null, restore: RestoreOptions_Deserialize | null, runs: JobRun_Serialize[] }
const jobs = new Map<string, FakeJob>()

export function recordJobRun(id: string | null | undefined, status: string, rows: number | null, log: string) {
  const j = id ? jobs.get(id) : undefined
  if (!j) return
  const run: JobRun_Serialize = { id: `run-${j.runs.length + 1}`, started_at: Date.now() - 1000, ended_at: Date.now(), status, rows, bytes: null, warnings: 0, log }
  j.runs.unshift(run)
  j.row.last_run = run
}

function save(name: string, kind: JobRow_Serialize['kind'], source: string | null, target: string | null, summary: string, m: { dbs: { id: string, name: string }[] }) {
  const n = name.trim()
  if (!n) fail('validation', 'Give the job a name of up to 80 characters.')
  if ([...jobs.values()].some(j => j.row.name === n)) fail('validation', `A job named “${n}” already exists. Pick another name.`)
  const id = `job-${jobs.size + 1}-${Date.now()}`
  const row: JobRow_Serialize = { id, name: n, kind, source_database_id: source, source_name: m.dbs.find(d => d.id === source)?.name ?? null, target_database_id: target, target_name: m.dbs.find(d => d.id === target)?.name ?? null, summary, created_at: Date.now(), last_run: null }
  jobs.set(id, { row, copy: null, restore: null, runs: [] })
  return jobs.get(id)!
}

const get = (id: string) => jobs.get(id) ?? fail('not_found', 'That job is no longer saved.')

export const jobsFake: Pick<Handlers, Keys> = {
  jobsList: () => [...jobs.values()].map(j => j.row).sort((a, b) => a.name.localeCompare(b.name)),
  jobRuns: a => get(a.jobId).runs,
  jobRename: (a) => {
    get(a.jobId).row.name = a.name.trim()
    return null
  },
  jobDelete: (a) => {
    jobs.delete(a.jobId)
    return null
  },
  jobSaveCopy: (a, { m }) => {
    const src = m.sqlConns.get(a.connId)?.db ?? null
    const dst = m.sqlConns.get(a.request.target_conn_id)?.db ?? null
    const masked = a.request.objects.some((o: { masks?: unknown[] | null }) => (o.masks ?? []).length)
    const j = save(a.name, 'copy', src, dst, `${a.request.objects.length} objects from ${m.dbs.find(d => d.id === src)?.name} to ${m.dbs.find(d => d.id === dst)?.name}${masked ? ', masked' : ''}`, m)
    j.copy = { ...a.request, target_conn_id: '' }
    return j.row
  },
  jobSaveRestore: (a, { m }) => {
    const db = m.sqlConns.get(a.connId)?.db ?? null
    const j = save(a.name, 'restore', null, db, `shop-2026-11-20.dump into ${m.dbs.find(d => d.id === db)?.name}`, m)
    j.restore = a.options
    return j.row
  },
  jobSaveBackup: (a, { m }) => {
    const db = m.sqlConns.get(a.connId)?.db ?? null
    return save(a.name, 'backup', db, null, `Back up ${m.dbs.find(d => d.id === db)?.name} to the folder backups`, m).row
  },
  jobSetup: (a) => {
    const j = get(a.jobId)
    return { kind: j.row.kind, source_database_id: j.row.source_database_id, target_database_id: j.row.target_database_id, copy: j.copy as never, restore: j.restore as never, file_name: j.row.kind === 'restore' ? 'shop-2026-11-20.dump' : null }
  },
  restorePickJob: (a, { m }) => {
    get(a.jobId)
    const db = m.dbs.find(d => d.id === m.sqlConns.get(a.connId)?.db)
    return dumpFor(db?.engine ?? 'postgres', 'clean', false)
  },
  jobBackupRun: (a, { channel, send, tag }) => {
    get(a.jobId)
    send(channel(a.progress), { rows: 0, bytes: 3 * 2 ** 20 })
    recordJobRun(a.jobId, 'ok', null, 'Saved /backups/app-nightly.dump')
    tag(`job-backup:${a.jobId}`)
    return { path: '/backups/app-nightly.dump', rows: 0, bytes: 3 * 2 ** 20 }
  },
}
