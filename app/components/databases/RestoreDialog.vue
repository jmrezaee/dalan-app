<script setup lang="ts">
// S48 "Restore a dump" (F31): a dump made anywhere, into this database or a new one. Rust reads the file (its path
// never reaches the window), says what is inside and what it won't restore, plans against the server, and holds the
// restore for the confirmation (typed for an existing database). The tool runs on the host, or here on a direct route.
import { ArrowLeft, FileArchive, FolderOpen, Server, TriangleAlert } from '@lucide/vue'
import { Button, buttonVariants } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Progress } from '@/components/ui/progress'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import type { Engine } from '~/types/bindings'
import { cn } from '@/lib/utils'

const props = defineProps<{ tabKey: string, dbName: string, database: string | null, environment: string, engine: Engine }>()
const store = useRestoreStore(props.tabKey)
const open = computed({ get: () => store.open, set: (v: boolean) => { if (!v) store.close() } })
const hostPick = ref(false)

const STEPS = [['file', 'File'], ['inside', 'What\'s inside'], ['target', 'Target and options'], ['review', 'Review'], ['progress', 'Restore']] as const
const stepIndex = computed(() => STEPS.findIndex(s => s[0] === (store.step === 'result' ? 'progress' : store.step)))
const fmt = (n: number | null | undefined) => Math.round(n ?? 0).toLocaleString('en-US')
const size = (raw: number | null | undefined) => {
  const n = raw ?? 0
  if (n >= 1 << 30) return `${(n / (1 << 30)).toFixed(1)} GB`
  if (n >= 1 << 20) return `${(n / (1 << 20)).toFixed(1)} MB`
  return `${Math.max(1, Math.round(n / 1024))} kB`
}
const mongo = computed(() => props.engine === 'mongodb')
const pg = computed(() => props.engine === 'postgres')
const production = computed(() => props.environment === 'production')
const envLabel = (e: string) => (e === 'none' ? 'no environment set' : e)

// ---- What's inside
const d = computed(() => store.dump)
const shown = computed(() => (d.value?.objects ?? []).slice(0, 300))
const hiddenCount = computed(() => (d.value ? d.value.objects.length - shown.value.length + (d.value.objects_more ?? 0) : 0))
const kinds = computed(() => {
  const m = new Map<string, number>()
  for (const o of d.value?.objects ?? []) m.set(o.kind, (m.get(o.kind) ?? 0) + 1)
  return [...m].map(([k, n]) => `${fmt(n)} ${k}${n === 1 ? '' : 's'}`).join(' · ')
})
const engineMismatch = computed(() => {
  const e = d.value?.engine
  if (!e) return ''
  const fits = e === 'mysql' ? props.engine === 'mysql' || props.engine === 'mariadb' : e === props.engine
  return fits ? '' : `This is a ${d.value!.format_label}; ${props.dbName} is ${props.engine === 'postgres' ? 'PostgreSQL' : props.engine === 'mongodb' ? 'MongoDB' : props.engine === 'mariadb' ? 'MariaDB' : 'MySQL'}. Open a database of the same engine to restore it.`
})

// ---- Target and options
const hasRoles = computed(() => d.value?.notes.some(n => n.code === 'roles') ?? false)
// Choosing what to restore: tables and collections can be ticked (everything else comes with them or stays).
const pickable = (kind: string) => ['table', 'table data', 'collection', 'time series'].includes(kind)
const pickNames = computed(() => (d.value?.objects ?? []).filter(o => pickable(o.kind)).map(o => o.name))
const isPicked = (name: string) => store.selected === null || store.selected.includes(name)
function togglePick(name: string) {
  const all = pickNames.value
  const cur = store.selected ?? [...all]
  const next = cur.includes(name) ? cur.filter(n => n !== name) : [...cur, name]
  store.selected = next.length === all.length ? null : next
}
const pickedCount = computed(() => (store.selected === null ? pickNames.value.length : store.selected.length))
const archive = computed(() => d.value?.format === 'pg_custom' || d.value?.format === 'pg_tar' || d.value?.format === 'pg_directory')
const schemas = computed(() => [...new Set((d.value?.objects ?? []).filter(o => o.name.includes('.') && o.kind !== 'trigger').map(o => o.name.split('.')[0]!))])
const jobsAllowed = computed(() => (d.value?.format === 'pg_custom' || d.value?.format === 'pg_directory') && !store.options.single_transaction)
watch(jobsAllowed, (ok) => { if (!ok) store.options.jobs = 1 })
watch(() => [store.options.content, store.options.existing], () => { if (store.options.content !== 'data') store.options.disable_triggers = false })
// After a restore into a new database: save it as a database and open it in a tab.
const sessions = useSessionsStore()
const opening = ref(false)
async function openNew() {
  const res = store.result
  if (!res) return
  opening.value = true
  try {
    const row = await store.saveDatabase(res.target_db)
    await useDatabasesStore().refresh()
    if (row.engine === 'mongodb') sessions.openMongo(row)
    else sessions.openSql(row)
    store.close()
  }
  catch (e) { store.error = e instanceof Error ? e.message : (e as { message?: string })?.message ?? 'Couldn\'t save it.' }
  opening.value = false
}
const canTargetThis = computed(() => !!props.database || pg.value)
const newNameOk = computed(() => store.options.target !== 'new' || store.options.newName.trim().length > 0)
watch(() => store.options.existing, (v) => { if (v === 'append') store.options.content = 'data' })

// ---- Review
const p = computed(() => store.plan)
const naming = ref(false)
const jobName = ref('')
const typedOk = computed(() => !p.value || p.value.level !== 'typed' || store.typed === p.value.typed_name)
// The review opens with Cancel focused: Enter must never start a restore by accident.
const cancelBtn = ref<{ $el: HTMLElement } | null>(null)
watch(() => store.step, (s) => { if (s === 'review') void nextTick(() => cancelBtn.value?.$el?.focus()) })

// ---- Progress
const pr = computed(() => store.progress)
const pct = computed(() => (pr.value && pr.value.total ? Math.min(100, Math.round(((pr.value.bytes ?? 0) / (pr.value.total ?? 1)) * 100)) : 0))
const rate = computed(() => {
  const s = (Date.now() - store.startedAt) / 1000
  return s > 1 && pr.value ? (pr.value.bytes ?? 0) / s : 0
})
const left = computed(() => {
  if (!pr.value || !rate.value) return ''
  const s = Math.max(0, ((pr.value.total ?? 0) - (pr.value.bytes ?? 0)) / rate.value)
  return s < 60 ? `about ${Math.ceil(s)} s left` : `about ${Math.ceil(s / 60)} min left`
})
const r = computed(() => store.result)
const radio = 'flex items-start gap-2 text-sm'
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="gap-0 p-0 sm:max-w-[640px]" :data-testid="tid('restore-dialog')" @escape-key-down="(e: Event) => store.step === 'progress' && e.preventDefault()" @pointer-down-outside="(e: Event) => e.preventDefault()">
      <DialogHeader class="gap-2 border-b border-border p-5 pb-3">
        <DialogTitle>Restore a dump into {{ dbName }}</DialogTitle>
        <DialogDescription class="sr-only">Pick a dump file, check what is inside, choose the target and confirm.</DialogDescription>
        <ol class="m-0 flex list-none flex-wrap gap-x-3 gap-y-1 p-0 text-xs" aria-label="Steps">
          <li v-for="(s, i) in STEPS" :key="s[0]" :aria-current="i === stepIndex ? 'step' : undefined" :class="cn(i === stepIndex ? 'font-medium text-text' : 'text-muted')">{{ i + 1 }}. {{ s[1] }}</li>
        </ol>
      </DialogHeader>

      <div class="max-h-[60vh] overflow-auto p-5 text-sm">
        <!-- 1. File -->
        <section v-if="store.step === 'file'" aria-label="File" class="flex flex-col gap-3">
          <p class="text-muted">Any dump works, not only Dalan's backups: pg_dump (custom, tar, directory or SQL), pg_dumpall, mysqldump, mariadb-dump and mongodump (archive or folder), also gzipped, zstd, bzip2, xz or zipped. Dalan reads what it is from the file itself.</p>
          <div class="flex flex-wrap gap-2">
            <Button :loading="store.busy" :data-testid="tid('restore-pick-file')" @click="store.pick(false)"><FileArchive class="size-4" aria-hidden="true" />Choose a file…</Button>
            <Button variant="secondary" :disabled="store.busy" :data-testid="tid('restore-pick-folder')" @click="store.pick(true)"><FolderOpen class="size-4" aria-hidden="true" />Choose a folder…</Button>
            <Button variant="secondary" :disabled="store.busy" :data-testid="tid('restore-pick-host')" @click="hostPick = true"><Server class="size-4" aria-hidden="true" />Choose on a host…</Button>
            <HostFolderPicker v-model:open="hostPick" mode="file" title="Restore a dump from a host" :on-pick="(p) => store.pickRemote(p.sftpId, p.hostId, p.path)" />
          </div>
          <p v-if="store.busy" role="status" class="text-xs text-muted">Reading the file… big dumps take a moment.</p>
        </section>

        <!-- 2. What's inside -->
        <section v-else-if="store.step === 'inside' && d" aria-label="What's inside" class="flex flex-col gap-3">
          <dl class="grid grid-cols-[120px_minmax(0,1fr)] gap-y-1 rounded-md border border-border bg-bg p-3 text-xs" data-part="source">
            <dt class="text-muted">File</dt><dd class="truncate font-mono" :title="d.file_name">{{ d.file_name }} · {{ size(d.bytes) }}</dd>
            <dt class="text-muted">Format</dt><dd>{{ d.format_label }}<span v-if="d.guessed" class="text-warning"> (a guess)</span><span v-if="d.compression" class="text-muted"> · {{ d.compression }}</span></dd>
            <template v-if="d.made_by"><dt class="text-muted">Made by</dt><dd>{{ d.made_by }}</dd></template>
            <template v-if="d.server_version"><dt class="text-muted">From</dt><dd>{{ d.server_version }}<span v-if="d.source_db"> · database {{ d.source_db }}</span></dd></template>
            <template v-if="d.created"><dt class="text-muted">Made on</dt><dd>{{ d.created }}</dd></template>
            <template v-if="d.encoding"><dt class="text-muted">Encoding</dt><dd>{{ d.encoding }}</dd></template>
          </dl>
          <p class="-mt-2 text-xs text-muted">Read from the file's first bytes, not its name.</p>
          <p v-if="d.refused" role="alert" class="flex items-start gap-2 rounded-md border border-danger/40 p-3 text-danger" data-part="refused"><TriangleAlert class="mt-0.5 size-4 shrink-0" aria-hidden="true" />{{ d.refused }}</p>
          <p v-else-if="engineMismatch" role="alert" class="text-danger" data-part="mismatch">{{ engineMismatch }}</p>
          <div>
            <h3 class="mb-1 text-xs font-medium text-muted">{{ kinds || 'No objects listed' }}</h3>
            <table v-if="shown.length" class="w-full table-fixed border-collapse text-xs" aria-label="Objects in the dump">
              <colgroup><col class="w-8"><col class="w-28"><col><col class="w-28"></colgroup>
              <thead class="sr-only"><tr><th>Restore</th><th>Kind</th><th>Name</th><th>Rows</th></tr></thead>
              <tbody>
                <tr v-for="(o, i) in shown" :key="i" class="border-t border-surface-raised">
                  <td class="h-7"><input v-if="pickable(o.kind) && !d.refused" type="checkbox" :checked="isPicked(o.name)" :aria-label="`Restore ${o.name}`" @change="togglePick(o.name)"></td>
                  <td class="text-muted">{{ o.kind }}</td>
                  <td class="truncate font-mono" :title="o.name"><span v-if="d.databases.length > 1 && o.database" class="text-muted">{{ o.database }} · </span>{{ o.name }}</td>
                  <td class="w-28 text-right tabular-nums text-muted">{{ o.rows === null ? '' : `${o.rows_exact ? '' : 'about '}${fmt(o.rows)} ${mongo ? 'docs' : 'rows'}` }}</td>
                </tr>
              </tbody>
            </table>
            <p v-if="hiddenCount > 0" class="mt-1 text-xs text-muted">and {{ fmt(hiddenCount) }} more</p>
            <div v-if="pickNames.length > 1 && !d.refused" class="mt-2 flex items-center gap-2 text-xs text-muted" data-part="picked">
              <span>{{ store.selected === null ? `All ${fmt(pickNames.length)} ${mongo ? 'collections' : 'tables'}` : `${fmt(pickedCount)} of ${fmt(pickNames.length)} ${mongo ? 'collections' : 'tables'}` }} will be restored{{ store.selected === null || mongo ? '' : ', with what belongs to them' }}.</span>
              <Button size="sm" variant="ghost" @click="store.selected = null">All</Button>
              <Button size="sm" variant="ghost" @click="store.selected = []">None</Button>
            </div>
          </div>
          <div v-if="d.notes.length">
            <h3 class="mb-1 text-xs font-medium text-muted">Things that will need attention</h3>
            <ul class="m-0 flex list-none flex-col gap-1 p-0 text-xs" data-part="notes"><li v-for="n in d.notes" :key="n.code" :class="n.code === 'superuser' || n.code === 'guessed' ? 'text-warning' : ''">{{ n.text }}</li></ul>
          </div>
        </section>

        <!-- 3. Target and options -->
        <section v-else-if="store.step === 'target' && d" aria-label="Target and options" class="flex flex-col gap-4">
          <fieldset class="flex flex-col gap-2">
            <legend class="mb-1 text-xs font-medium text-muted">Restore into</legend>
            <label :class="radio"><input v-model="store.options.target" type="radio" value="this" :disabled="!canTargetThis">{{ database ?? dbName }} <span class="text-muted">(this database{{ production ? ', production' : '' }})</span></label>
            <label :class="radio"><input v-model="store.options.target" type="radio" value="new">A new database on the same server</label>
            <Input v-if="store.options.target === 'new'" v-model="store.options.newName" class="ml-6 h-8 w-64 font-mono text-xs" placeholder="name" aria-label="New database name" autocomplete="off" spellcheck="false" :data-testid="tid('restore-new-name')" />
            <p v-if="production && store.options.target === 'this'" class="ml-6 text-xs text-warning">Restoring into a production database needs it switched to Open first. Restore into a new database instead.</p>
          </fieldset>
          <label v-if="d.databases.length > 1" class="flex items-center gap-2 text-xs text-muted">Database from the dump
            <select v-model="store.options.source_database" class="h-7 rounded-md border border-border-input bg-bg px-2 font-mono text-xs text-text"><option v-for="x in d.databases" :key="x" :value="x">{{ x }}</option></select>
          </label>
          <fieldset class="flex flex-col gap-2">
            <legend class="mb-1 text-xs font-medium text-muted">Objects that already exist</legend>
            <label :class="radio"><input v-model="store.options.existing" type="radio" value="stop">Stop if anything already exists</label>
            <label :class="radio"><input v-model="store.options.existing" type="radio" value="replace">Drop and replace what the dump contains</label>
            <label :class="radio"><input v-model="store.options.existing" type="radio" value="append">Add rows to the existing tables (data only)</label>
          </fieldset>
          <fieldset v-if="!mongo" class="flex flex-col gap-2">
            <legend class="mb-1 text-xs font-medium text-muted">What to restore</legend>
            <div class="flex flex-wrap gap-4">
              <label :class="radio"><input v-model="store.options.content" type="radio" value="all" :disabled="store.options.existing === 'append'">Schema and data</label>
              <label :class="radio"><input v-model="store.options.content" type="radio" value="schema" :disabled="store.options.existing === 'append'">Schema only</label>
              <label :class="radio"><input v-model="store.options.content" type="radio" value="data">Data only</label>
            </div>
          </fieldset>
          <fieldset class="flex flex-col gap-2">
            <legend class="mb-1 text-xs font-medium text-muted">If something fails</legend>
            <label :class="radio"><input v-model="store.options.single_transaction" type="radio" :value="true">{{ pg ? 'Stop and undo everything (one transaction)' : 'Stop at the first error' }}</label>
            <label :class="radio"><input v-model="store.options.single_transaction" type="radio" :value="false">Keep going and report the errors</label>
          </fieldset>
          <div v-if="!mongo" class="flex flex-col gap-2">
            <label :class="radio"><input v-model="store.options.skip_owner_acl" type="checkbox">Skip ownership and privileges (everything belongs to the user you sign in as)</label>
            <label v-if="hasRoles" :class="radio"><input v-model="store.options.include_roles" type="checkbox">Include roles (never superusers)</label>
          </div>
          <fieldset v-if="pg && schemas.length" class="flex flex-col gap-2">
            <legend class="mb-1 text-xs font-medium text-muted">Rename a schema on the way (optional)</legend>
            <div class="flex flex-wrap items-center gap-2 text-xs">
              <select v-model="store.options.renameFrom" class="h-7 rounded-md border border-border-input bg-bg px-2 font-mono text-text" aria-label="Schema in the dump"><option value="">none</option><option v-for="x in schemas" :key="x" :value="x">{{ x }}</option></select>
              <span class="text-muted">becomes</span>
              <Input v-model="store.options.renameTo" :disabled="!store.options.renameFrom" class="h-7 w-40 font-mono text-xs" aria-label="New schema name" autocomplete="off" spellcheck="false" />
            </div>
          </fieldset>
          <div v-if="pg && archive" class="flex flex-col gap-2">
            <label class="flex items-center gap-2 text-sm">Parallel jobs
              <input v-model.number="store.options.jobs" type="number" min="1" max="16" :disabled="!jobsAllowed" class="h-7 w-16 rounded-md border border-border-input bg-bg px-2 text-xs" aria-label="Parallel jobs">
              <span class="text-xs text-muted">{{ jobsAllowed ? 'faster on big dumps; the archive goes to the host as a file first' : 'for custom and directory archives that keep going on errors' }}</span>
            </label>
            <label :class="radio"><input v-model="store.options.disable_triggers" type="checkbox" :disabled="store.options.content !== 'data'">Turn triggers off while loading (data only; needs a superuser)</label>
          </div>
        </section>

        <!-- 4. Review -->
        <section v-else-if="store.step === 'review' && p" aria-label="Review" class="flex flex-col gap-3" :data-testid="tid('restore-review')">
          <p v-if="p.blocked" role="alert" class="rounded-md border border-warning/40 p-3 text-warning" data-part="blocked">{{ p.blocked }}</p>
          <dl class="grid grid-cols-[120px_minmax(0,1fr)] gap-y-1 rounded-md border border-border bg-bg p-3 text-xs">
            <dt class="text-muted">Into</dt><dd><b class="font-mono">{{ p.target_db }}</b>{{ p.new_database ? ' (new)' : '' }} on {{ p.saved_name }} · <span :class="p.environment === 'production' ? 'text-warning' : 'text-muted'">{{ envLabel(p.environment) }}</span></dd>
            <dt class="text-muted">From</dt><dd class="truncate font-mono">{{ d?.file_name }} · {{ size(p.bytes) }}</dd>
            <dt class="text-muted">Contents</dt><dd>{{ fmt(p.objects) }} objects{{ p.rows ? ` · ${fmt(p.rows)} ${mongo ? 'documents' : 'rows'}` : '' }}</dd>
            <dt class="text-muted">Runs</dt><dd>{{ p.tool_version || p.tool }} on {{ p.runs_on }}{{ p.upload ? ', after copying the folder there (removed afterwards)' : '' }}</dd>
            <dt class="text-muted">If it fails</dt><dd>{{ p.atomic ? 'Nothing changes: it runs in one transaction.' : 'What ran so far stays.' }}</dd>
          </dl>
          <div v-if="p.replaces.length" data-part="replaces">
            <h3 class="mb-1 text-xs font-medium text-danger">Dropped and replaced ({{ fmt(p.replaces.length) }})</h3>
            <p class="font-mono text-xs">{{ p.replaces.slice(0, 20).join(', ') }}<span v-if="p.replaces.length > 20" class="text-muted"> and {{ p.replaces.length - 20 }} more</span></p>
          </div>
          <div v-if="p.skipped.length">
            <h3 class="mb-1 text-xs font-medium text-muted">Left out</h3>
            <ul class="m-0 list-disc pl-4 text-xs" data-part="skipped"><li v-for="s in p.skipped" :key="s">{{ s }}</li></ul>
          </div>
          <ul v-if="p.warnings.length" class="m-0 flex list-none flex-col gap-1 p-0 text-xs text-warning" data-part="warnings"><li v-for="w in p.warnings" :key="w">{{ w }}</li></ul>
          <div v-if="!store.jobId && !p.blocked" class="flex items-center gap-2 text-xs">
            <template v-if="naming"><Input v-model="jobName" class="h-7 w-56 text-xs" placeholder="Restore last night's dump" aria-label="Job name" /><Button size="sm" :disabled="!jobName.trim()" :data-testid="tid('restore-save-job')" @click="store.saveJob(jobName); naming = false">Save</Button></template>
            <Button v-else size="sm" variant="ghost" @click="naming = true">Save as a job…</Button>
          </div>
          <p v-if="store.saved" role="status" class="text-xs text-success">{{ store.saved }}</p>
          <label v-if="p.level === 'typed' && !p.blocked" class="flex flex-col gap-1 text-xs text-muted">Type {{ p.typed_name }} to confirm
            <Input v-model="store.typed" class="font-mono text-xs text-text" autocomplete="off" spellcheck="false" :data-testid="tid('restore-typed')" @keydown.enter.prevent="typedOk && store.commit()" />
          </label>
        </section>

        <!-- 5. Progress -->
        <section v-else-if="store.step === 'progress'" aria-label="Restore" class="flex flex-col gap-3" :data-testid="tid('restore-progress')">
          <p role="status" class="text-sm">{{ pr?.phase === 'upload' ? `Copying the folder to ${p?.runs_on}…` : `Restoring into ${p?.target_db}…` }}</p>
          <Progress :model-value="pct" aria-label="Progress" />
          <p class="text-xs text-muted tabular-nums">{{ size(pr?.bytes) }} of {{ size(pr?.total ?? p?.bytes) }}<span v-if="rate"> · {{ size(rate) }}/s · {{ left }}</span><span v-if="pr?.objects"> · {{ fmt(pr.objects) }} objects</span></p>
          <p v-if="pr?.current" class="truncate font-mono text-xs text-muted" :title="pr.current">{{ pr.current }}</p>
        </section>

        <!-- 6. Result -->
        <section v-else-if="store.step === 'result' && r" aria-label="Result" class="flex flex-col gap-3" :data-testid="tid('restore-result')">
          <p v-if="r.outcome === 'done'" role="status" class="text-success" data-part="outcome">Restored into {{ r.target_db }}: {{ fmt(r.objects) }} objects in {{ Math.max(1, Math.round(r.seconds ?? 0)) }} s.</p>
          <p v-else-if="r.outcome === 'stopped'" role="status" data-part="outcome">{{ r.nothing_changed ? 'Nothing changed.' : `Stopped after ${fmt(r.objects)} objects; these are in the database now.` }}</p>
          <p v-else role="alert" class="text-danger" data-part="outcome">{{ r.message }}<template v-if="r.nothing_changed"> Nothing changed.</template></p>
          <p v-if="r.removed_new_database" class="text-xs text-muted">The new database {{ r.target_db }} was removed again.</p>
          <ul v-if="r.outcome !== 'done' && !r.nothing_changed && r.done.length" class="m-0 max-h-32 list-none overflow-auto p-0 font-mono text-xs text-muted"><li v-for="x in r.done" :key="x">{{ x }}</li></ul>
          <div v-if="r.warnings.length">
            <h3 class="mb-1 text-xs font-medium text-muted">Warnings from {{ p?.tool }}</h3>
            <ul class="m-0 flex max-h-48 list-none flex-col gap-1 overflow-auto p-0 text-xs" data-part="warnings"><li v-for="w in r.warnings" :key="w.text" class="break-words">{{ w.text }}<span v-if="(w.count ?? 1) > 1" class="text-muted"> ×{{ fmt(w.count) }}</span></li></ul>
          </div>
        </section>

        <p v-if="store.error" role="alert" class="mt-3 text-sm text-danger" data-part="error">{{ store.error }}</p>
      </div>

      <DialogFooter class="border-t border-border p-4">
        <Button v-if="['inside', 'target', 'review'].includes(store.step)" variant="ghost" class="mr-auto" @click="store.step = store.step === 'review' ? 'target' : store.step === 'target' ? 'inside' : 'file'"><ArrowLeft class="size-4" aria-hidden="true" />Back</Button>
        <template v-if="store.step === 'progress'">
          <Button variant="secondary" :data-testid="tid('restore-stop')" @click="store.stop()">Stop</Button>
        </template>
        <template v-else-if="store.step === 'result'">
          <Button v-if="r?.outcome === 'done' && p?.new_database" variant="secondary" :loading="opening" :data-testid="tid('restore-open-new')" @click="openNew">Open {{ r.target_db }}</Button>
          <Button @click="store.close()">Close</Button>
        </template>
        <template v-else>
          <Button ref="cancelBtn" variant="secondary" @click="store.close()">Cancel</Button>
          <Button v-if="store.step === 'inside'" :disabled="!!d?.refused || !!engineMismatch || store.selected?.length === 0" @click="store.step = 'target'">Next</Button>
          <Button v-if="store.step === 'target'" :disabled="!newNameOk" :loading="store.busy" :data-testid="tid('restore-next')" @click="store.review()">Review</Button>
          <Button v-if="store.step === 'review'" :class="p?.level === 'typed' ? buttonVariants({ variant: 'destructive' }) : ''" :disabled="!p?.pending_id || !typedOk" :data-testid="tid('restore-confirm')" @click="store.commit()">Restore</Button>
        </template>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
