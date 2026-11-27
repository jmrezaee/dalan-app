// Saved jobs (D14, E1). The fake keeps jobs and runs in memory; a run is recorded when a copy, restore or backup
// that carries the job's id ends. Nothing here holds a password, like the real thing.
import { fail, type Handlers } from '../types'
import { dumpFor } from './restore'
import type { Every, JobRow_Serialize, JobRun_Serialize, CopyRequest_Deserialize, RestoreOptions_Deserialize } from '../../../app/types/bindings'

type Keys = 'jobsList' | 'jobRuns' | 'jobRename' | 'jobDelete' | 'jobSaveCopy' | 'jobSaveRestore' | 'jobSaveBackup' | 'jobSetup' | 'restorePickJob' | 'jobBackupRun' | 'jobSetSchedule' | 'jobSchedulePreview'

interface FakeJob { row: JobRow_Serialize, copy: CopyRequest_Deserialize | null, restore: RestoreOptions_Deserialize | null, runs: JobRun_Serialize[] }
const jobs = new Map<string, FakeJob>()

export function recordJobRun(id: string | null | undefined, status: string, rows: number | null, log: string, warnings = 0) {
  const j = id ? jobs.get(id) : undefined
  if (!j) return
  const run: JobRun_Serialize = { id: `run-${j.runs.length + 1}`, started_at: Date.now() - 1000, ended_at: Date.now(), status, rows, bytes: null, warnings, log }
  j.runs.unshift(run)
  j.row.last_run = run
  j.row.missed_before_last = status !== 'missed' && j.runs[1]?.status === 'missed' ? j.runs[1].warnings : null
}

/** D15: a scheduled run, as Rust's scheduler records it (with the slots it missed while locked first). */
export function scheduledRun(name: string, status: string, missed: number) {
  const j = [...jobs.values()].find(x => x.row.name === name)
  if (!j) throw new Error(`No fake job ${name}`)
  if (missed) recordJobRun(j.row.id, 'missed', null, `Missed ${missed} runs while Dalan was closed or locked; ran once at unlock.`, missed)
  recordJobRun(j.row.id, status, null, status === 'ok' ? 'Saved /backups/app-nightly.dump' : 'Couldn\'t reach bastion: its host key isn\'t trusted yet.')
  return j.row.id
}

const DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday']
const pad = (n: number) => String(n).padStart(2, '0')
function describe(e: Every): string {
  if (e.every === 'cron') {
    if (e.expr.trim().split(/\s+/).length !== 5) fail('validation', 'A cron schedule has five fields: minute hour day-of-month month weekday (for example 30 2 * * 1-5).')
    return `Cron ${e.expr.trim()}`
  }
  if (e.every === 'hour') return `Every hour at :${pad(e.minute)}`
  if (e.every === 'day') return `Every day at ${pad(e.hour)}:${pad(e.minute)}`
  return `Every ${DAYS[e.weekday]} at ${pad(e.hour)}:${pad(e.minute)}`
}

function save(name: string, kind: JobRow_Serialize['kind'], source: string | null, target: string | null, summary: string, m: { dbs: { id: string, name: string, environment?: string }[] }) {
  const n = name.trim()
  if (!n) fail('validation', 'Give the job a name of up to 80 characters.')
  if ([...jobs.values()].some(j => j.row.name === n)) fail('validation', `A job named “${n}” already exists. Pick another name.`)
  const id = `job-${jobs.size + 1}-${Date.now()}`
  const env = m.dbs.find(d => d.id === target)?.environment
  const noSchedule = kind === 'restore' ? 'Restores run only when you start them.' : kind === 'copy' && env === 'production' ? 'Its target is production, which always asks for the typed name: it can\'t run on a schedule.' : null
  const row: JobRow_Serialize = { id, name: n, kind, source_database_id: source, source_name: m.dbs.find(d => d.id === source)?.name ?? null, target_database_id: target, target_name: m.dbs.find(d => d.id === target)?.name ?? null, summary, created_at: Date.now(), last_run: null, schedule: null, schedule_text: null, next_run_at: null, retention: null, no_schedule: noSchedule, missed_before_last: null }
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
  jobSaveBackup: (a, { m, tag }) => {
    const db = m.sqlConns.get(a.connId)?.db ?? null
    const name = m.dbs.find(d => d.id === db)?.name
    if (a.host) {
      tag(`job-backup-host:${a.host.host_id}:${a.host.dir}`)
      return save(a.name, 'backup', db, null, `Back up ${name} to ${a.host.dir} on ${m.hosts.find(h => h.id === a.host!.host_id)?.label}`, m).row
    }
    return save(a.name, 'backup', db, null, `Back up ${name} to the folder backups`, m).row
  },
  jobSetSchedule: (a) => {
    const j = get(a.jobId)
    if (a.schedule && j.row.no_schedule) fail('validation', j.row.no_schedule)
    j.row.schedule = a.schedule ? { ...a.schedule } as never : null
    j.row.schedule_text = a.schedule ? describe(a.schedule) : null
    j.row.next_run_at = a.schedule ? Date.now() + 3600_000 : null
    const r = a.retention
    j.row.retention = j.row.kind === 'backup' && r && (r.keep_last != null || r.keep_days != null) ? { keep_last: r.keep_last ?? null, keep_days: r.keep_days ?? null } : null
    return j.row
  },
  jobSchedulePreview: a => ({ text: describe(a.every), next: [1, 2, 3].map(h => Date.now() + h * 3600_000) }),
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
