<script setup lang="ts">
// Jobs (D14, E1): saved restores, copies and backups. Run opens the database's tab and goes through the same review as
// the first time (the confirmation is built fresh: nothing approved earlier carries over). History shows the last 50
// runs with their logs. A job never holds a password.
import { ArchiveRestore, CalendarClock, Copy, HardDriveDownload, ListChecks, Pencil, Play, Trash2 } from '@lucide/vue'
import { Button, buttonVariants } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { JobRow_Serialize as JobRow } from '~/types/bindings'

const jobs = useJobsStore()
onMounted(() => { void jobs.refresh(); jobs.failedUnseen = false })
const ICON = { copy: Copy, restore: ArchiveRestore, backup: HardDriveDownload } as const
const KIND = { copy: 'Copy', restore: 'Restore', backup: 'Backup' } as const
const when = (ms: number | null | undefined) => (ms ? new Date(ms).toLocaleString() : '')
const STATUS: Record<string, string> = { ok: 'text-success', failed: 'text-danger', stopped: 'text-warning', missed: 'text-warning' }

const open = ref<string | null>(null)
// D15: the schedule dialog, for one job at a time.
const scheduling = ref<JobRow | null>(null)
const scheduleOpen = computed({ get: () => scheduling.value !== null, set: (v: boolean) => { if (!v) scheduling.value = null } })
const time = (ms: number | null | undefined) => (ms ? new Date(ms).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' }) : '')
const keeps = (j: JobRow) => {
  const r = j.retention
  if (!r) return ''
  return [r.keep_last != null && `the last ${r.keep_last}`, r.keep_days != null && `${r.keep_days} days`].filter(Boolean).join(' or ')
}
async function toggleHistory(j: JobRow) {
  open.value = open.value === j.id ? null : j.id
  if (open.value) await jobs.loadRuns(j.id)
}
// The newest job whose last run didn't finish well (the board's banner).
const failed = computed(() => jobs.rows.filter(j => j.last_run && ['failed', 'stopped'].includes(j.last_run.status)).sort((a, b) => (b.last_run!.started_at ?? 0) - (a.last_run!.started_at ?? 0))[0] ?? null)
const renaming = reactive({ id: '', name: '' })
async function rename() {
  try { await jobs.rename(renaming.id, renaming.name); renaming.id = '' }
  catch (e) { jobs.error = (e as { message?: string })?.message ?? 'Couldn\'t rename it.' }
}
const toDelete = ref<JobRow | null>(null)
const confirmOpen = computed({ get: () => toDelete.value !== null, set: (v: boolean) => { if (!v) toDelete.value = null } })
async function remove() {
  const j = toDelete.value
  toDelete.value = null
  if (j) await jobs.remove(j.id)
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col" :data-testid="tid('jobs-view')">
    <header class="flex h-14 shrink-0 items-center gap-3 border-b border-border px-5">
      <h1 class="text-lg font-semibold">Jobs</h1>
      <p class="hidden truncate text-sm text-muted md:block">Saved restores, copies and backups you can run again.</p>
      <span class="ml-auto shrink-0 rounded-full bg-surface-raised px-2 py-0.5 text-xs text-muted">Schedules run while Dalan is open and unlocked</span>
    </header>
    <div class="min-h-0 flex-1 overflow-auto p-5">
      <p v-if="jobs.error" role="alert" class="mb-3 text-sm text-danger">{{ jobs.error }}</p>
      <div v-if="failed" role="alert" class="mb-3 flex max-w-[960px] items-center gap-3 rounded-md border border-danger/40 px-3 py-2 text-sm text-danger" data-part="failed-banner">
        <span class="min-w-0 flex-1">The last run of “{{ failed.name }}” {{ failed.last_run?.status === 'stopped' ? 'was cut off' : 'failed' }} at {{ time(failed.last_run?.started_at) }}. Its history says why.</span>
        <Button size="sm" variant="secondary" @click="open !== failed.id && toggleHistory(failed)">Open history</Button>
      </div>
      <EmptyState v-if="!jobs.rows.length" :icon="ListChecks" title="No saved jobs yet" text="Set up a copy, a restore or a backup in a database's tab, then choose “Save as a job…”. It runs again from here, asking the same questions." />
      <ul v-else class="m-0 flex max-w-[960px] list-none flex-col gap-2 p-0" aria-label="Saved jobs">
        <li v-for="j in jobs.rows" :key="j.id" class="rounded-lg border border-border bg-surface p-3 text-sm" :data-testid="tid(`job-${j.name}`)">
          <div class="flex items-center gap-3">
            <component :is="ICON[j.kind]" class="size-4 shrink-0 text-muted" aria-hidden="true" />
            <div class="min-w-0 flex-1">
              <template v-if="renaming.id === j.id">
                <Input v-model="renaming.name" class="h-7 w-64 text-sm" aria-label="Job name" @keydown.enter.prevent="rename" />
              </template>
              <p v-else class="font-medium">{{ j.name }} <span class="text-xs font-normal text-muted">{{ KIND[j.kind] }}</span></p>
              <p class="truncate text-xs text-muted" :title="j.summary">{{ j.summary }}</p>
              <p class="text-xs text-muted" data-part="last-run">
                <template v-if="jobs.running.has(j.id)"><span class="text-accent">Running now…</span></template>
                <template v-else-if="j.last_run">Last run {{ when(j.last_run.started_at) }} · <span :class="STATUS[j.last_run.status]">{{ j.last_run.status }}</span></template>
                <template v-else>Never run</template>
              </p>
              <p v-if="j.missed_before_last && j.last_run" class="text-xs text-warning" data-part="missed">Missed {{ j.missed_before_last }} {{ j.missed_before_last === 1 ? 'run' : 'runs' }} while closed or locked; ran at {{ time(j.last_run.started_at) }}</p>
              <p v-if="j.schedule_text" class="text-xs text-muted" data-part="schedule">{{ j.schedule_text }}<template v-if="j.next_run_at"> · next {{ when(j.next_run_at) }}</template><template v-if="keeps(j)"> · keeps {{ keeps(j) }}</template></p>
              <p v-else-if="j.no_schedule && j.kind === 'copy'" class="text-xs text-muted">{{ j.no_schedule }}</p>
            </div>
            <Button size="sm" :data-testid="tid('job-run')" @click="jobs.run(j)"><Play class="size-3.5" aria-hidden="true" />Run</Button>
            <Button v-if="!j.no_schedule" size="sm" variant="ghost" :data-testid="tid('job-schedule')" @click="scheduling = j"><CalendarClock class="size-3.5" aria-hidden="true" />Schedule…</Button>
            <Button size="sm" variant="ghost" :aria-expanded="open === j.id" @click="toggleHistory(j)">History</Button>
            <Button v-if="renaming.id === j.id" size="sm" variant="secondary" @click="rename">Save</Button>
            <Button v-else size="sm" variant="ghost" :aria-label="`Rename ${j.name}`" @click="Object.assign(renaming, { id: j.id, name: j.name })"><Pencil class="size-3.5" aria-hidden="true" /></Button>
            <Button size="sm" variant="ghost" :aria-label="`Delete ${j.name}`" @click="toDelete = j"><Trash2 class="size-3.5" aria-hidden="true" /></Button>
          </div>
          <div v-if="open === j.id" class="mt-2 border-t border-border pt-2" :aria-label="`Runs of ${j.name}`">
            <p v-if="!(jobs.runs[j.id] ?? []).length" class="text-xs text-muted">No runs yet.</p>
            <details v-for="r in jobs.runs[j.id] ?? []" :key="r.id" class="text-xs">
              <summary class="cursor-default"><span :class="STATUS[r.status]">{{ r.status }}</span> · {{ when(r.started_at) }}<span v-if="r.rows !== null"> · {{ Math.round(r.rows ?? 0).toLocaleString('en-US') }} rows</span><span v-if="r.warnings"> · {{ r.warnings }} warnings</span></summary>
              <pre class="mt-1 whitespace-pre-wrap break-all rounded-md border border-border bg-bg p-2 font-mono">{{ r.log || '(no log)' }}</pre>
            </details>
          </div>
        </li>
      </ul>
    </div>
    <ScheduleDialog v-if="scheduling" v-model:open="scheduleOpen" :job="scheduling" />
    <AlertDialog v-model:open="confirmOpen">
      <AlertDialogContent v-if="toDelete">
        <AlertDialogHeader>
          <AlertDialogTitle>Delete the job “{{ toDelete.name }}”?</AlertDialogTitle>
          <AlertDialogDescription>Its run history goes with it. The databases and any files it made stay.</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter><AlertDialogCancel>Cancel</AlertDialogCancel><Button :class="buttonVariants({ variant: 'destructive' })" @click="remove">Delete job</Button></AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
