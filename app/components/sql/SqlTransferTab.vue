<script setup lang="ts">
// S43 "Import, export and backup": three panels. Export streams a table to a file Rust writes after the system save
// dialog; Import maps a CSV onto a table and loads it in one transaction after a review; Backup runs the dump tool
// on the host (or here) and restore is always typed. The window never reads or writes a file: Rust does, and shows
// progress.
import { Button, buttonVariants } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { BackupInfo_Serialize as BackupInfo, CsvPreview, ExportFormat, CsvImportPending as ImportPending, RestorePending, TransferProgress } from '~/types/bindings'
import { cn } from '@/lib/utils'

const props = defineProps<{ tabKey: string }>()
const store = useSqlStore(props.tabKey)
const databases = useDatabasesStore()
const db = computed(() => databases.rows.find(d => d.id === store.dbId) ?? null)
const message = (e: unknown, fallback: string) => (e instanceof Error ? e.message : (e as { message?: string })?.message ?? fallback)
const fmt = (n: number | null) => Math.round(n ?? 0).toLocaleString('en-US')
const mb = (raw: number | null) => { const n = raw ?? 0; return n >= 1 << 20 ? `${(n / (1 << 20)).toFixed(1)} MB` : `${Math.round(n / 1024)} kB` }

const tables = computed(() => store.schemas.flatMap(s => (s.objects ?? []).filter(o => o.kind === 'table' || o.kind === 'matview').map(o => ({ schema: s.name, name: o.name, id: `${s.name}.${o.name}` }))))
const pick = ref('')
watch(tables, (t) => { if (!pick.value && t.length) pick.value = store.current ? `${store.current.schema}.${store.current.table}` : t[0]!.id }, { immediate: true })
const target = computed(() => tables.value.find(t => t.id === pick.value) ?? null)

// ---- Export
const FORMATS: { id: ExportFormat, label: string }[] = [{ id: 'csv', label: 'CSV' }, { id: 'tsv', label: 'TSV' }, { id: 'json', label: 'JSON' }, { id: 'sql', label: 'SQL inserts' }]
const exp = reactive({ format: 'csv' as ExportFormat, header: true, filter: '', running: false, id: '', rows: 0, bytes: 0, done: '', error: '' })
async function doExport() {
  const t = target.value
  if (!t || exp.running) return
  Object.assign(exp, { running: true, id: store.newTransferId(), rows: 0, bytes: 0, done: '', error: '' })
  try {
    const r = await store.exportTable(exp.id, t.schema, t.name, exp.filter, exp.format, exp.header, (p: TransferProgress) => { exp.rows = p.rows ?? 0; exp.bytes = p.bytes ?? 0 })
    exp.done = r ? `Saved ${r.path} · ${fmt(r.rows ?? 0)} rows · ${mb(r.bytes ?? 0)}` : ''
  }
  catch (e) { exp.error = message(e, 'Couldn\'t export.') }
  exp.running = false
}

// ---- Import
const imp = reactive({ fileId: '', name: '', delimiter: ',', header: true, preview: null as CsvPreview | null, mapping: [] as (string | null)[], pending: null as ImportPending | null, error: '', done: '', busy: false })
async function chooseFile() {
  imp.error = ''
  imp.done = ''
  try {
    const f = await store.importPick()
    if (!f) return
    imp.fileId = f.id
    imp.name = f.name
    await loadPreview()
  }
  catch (e) { imp.error = message(e, 'Couldn\'t open that file.') }
}
async function loadPreview() {
  const t = target.value
  if (!t || !imp.fileId) return
  try {
    imp.preview = await store.importPreview(t.schema, t.name, imp.fileId, imp.delimiter, imp.header)
    imp.mapping = [...imp.preview.suggested]
    imp.error = ''
  }
  catch (e) { imp.error = message(e, 'Couldn\'t read that file.') }
}
watch([pick, () => imp.delimiter, () => imp.header], () => { void loadPreview() })
async function reviewImport() {
  const t = target.value
  if (!t) return
  try { imp.pending = await store.importPrepare(t.schema, t.name, imp.fileId, imp.delimiter, imp.header, imp.mapping); imp.error = '' }
  catch (e) { imp.error = message(e, 'Couldn\'t prepare the import.') }
}
async function commitImport() {
  const p = imp.pending
  if (!p) return
  imp.busy = true
  try {
    const n = await store.importCommit(p.id)
    imp.pending = null
    imp.done = `Loaded ${fmt(n)} rows into ${pick.value}.`
    imp.preview = null
    imp.fileId = ''
  }
  catch (e) { imp.error = message(e, 'Couldn\'t load that.'); imp.pending = null }
  imp.busy = false
}
async function cancelImport() {
  const p = imp.pending
  imp.pending = null
  if (p) await store.importDiscard(p.id)
}
const importOpen = computed({ get: () => imp.pending !== null, set: (v: boolean) => { if (!v) void cancelImport() } })

// ---- Backup and restore
const bk = reactive({ info: null as BackupInfo | null, running: false, id: '', bytes: 0, done: '', error: '', restoring: null as RestorePending | null, typed: '', kind: '' as '' | 'backup' | 'restore' })
async function check() {
  try { bk.info = await store.backupCheck() }
  catch (e) { bk.error = message(e, 'Couldn\'t check the tool.') }
}
onMounted(check)
async function backup() {
  if (bk.running) return
  Object.assign(bk, { running: true, kind: 'backup', id: store.newTransferId(), bytes: 0, done: '', error: '' })
  try {
    const r = await store.backupStart(bk.id, (p: TransferProgress) => { bk.bytes = p.bytes ?? 0 })
    bk.done = r ? `Saved ${r.path} · ${mb(r.bytes ?? 0)}` : ''
  }
  catch (e) { bk.error = message(e, 'The backup failed.') }
  bk.running = false
}
async function startRestore() {
  bk.error = ''
  bk.done = ''
  try { bk.restoring = await store.restorePrepare(); bk.typed = '' }
  catch (e) { bk.error = message(e, 'Couldn\'t prepare the restore.') }
}
async function doRestore() {
  const r = bk.restoring
  if (!r) return
  const typed = bk.typed
  bk.restoring = null
  Object.assign(bk, { running: true, kind: 'restore', id: store.newTransferId(), bytes: 0, done: '', error: '' })
  try {
    await store.restoreCommit(r.id, typed, bk.id, (p: TransferProgress) => { bk.bytes = p.bytes ?? 0 })
    bk.done = `Restored ${r.file_name} into ${r.db_name}.`
  }
  catch (e) { bk.error = message(e, 'The restore failed.') }
  bk.running = false
}
const restoreOpen = computed({ get: () => bk.restoring !== null, set: (v: boolean) => { if (!v && bk.restoring) { void store.restoreDiscard(bk.restoring.id); bk.restoring = null } } })
const panel = 'flex flex-col gap-3 rounded-lg border border-border bg-surface p-4 text-sm'
</script>

<template>
  <div class="min-h-0 flex-1 overflow-auto p-4" :data-testid="tid('sql-transfer')">
    <div class="mb-3 flex items-center gap-2">
      <label for="transfer-table" class="text-xs text-muted">Table</label>
      <select id="transfer-table" v-model="pick" class="h-7 rounded-md border border-border-input bg-bg px-2 font-mono text-xs"><option v-for="t in tables" :key="t.id" :value="t.id">{{ t.id }}</option></select>
    </div>
    <div class="grid gap-4 lg:grid-cols-3">
      <section :class="panel" aria-label="Export">
        <h2 class="text-base font-semibold">Export</h2>
        <div class="flex flex-wrap gap-1" role="group" aria-label="Format"><button v-for="f in FORMATS" :key="f.id" type="button" :aria-pressed="exp.format === f.id" :class="cn(buttonVariants({ variant: exp.format === f.id ? 'secondary' : 'ghost', size: 'sm' }))" @click="exp.format = f.id">{{ f.label }}</button></div>
        <label class="flex flex-col gap-1 text-xs text-muted">Only rows where (optional)<Input v-model="exp.filter" class="h-7 font-mono text-xs text-text" placeholder="status = 'open'" autocomplete="off" /></label>
        <label v-if="exp.format === 'csv' || exp.format === 'tsv'" class="flex items-center gap-2 text-xs"><input v-model="exp.header" type="checkbox">Header row</label>
        <p class="text-xs text-muted">Saved on this computer. Big exports stream to the file instead of loading into the window.</p>
        <p v-if="exp.running" role="status" class="text-xs text-muted" data-part="progress">Writing… {{ fmt(exp.rows) }} rows · {{ mb(exp.bytes) }}</p>
        <p v-if="exp.done" role="status" class="text-xs text-success" data-part="done">{{ exp.done }}</p>
        <p v-if="exp.error" role="alert" class="text-xs text-danger">{{ exp.error }}</p>
        <div class="mt-auto flex justify-end gap-2"><Button v-if="exp.running" variant="secondary" @click="store.stopTransfer(exp.id)">Stop</Button><Button :disabled="!target || exp.running" @click="doExport">Save as…</Button></div>
      </section>

      <section :class="panel" aria-label="Import">
        <h2 class="text-base font-semibold">Import into {{ target?.name ?? 'a table' }}</h2>
        <template v-if="imp.preview">
          <p class="text-xs text-muted">{{ imp.name }} · {{ fmt(imp.preview.total_rows) }} rows</p>
          <div class="flex gap-3 text-xs">
            <label class="flex items-center gap-1.5 text-muted">Separator<select v-model="imp.delimiter" class="h-7 rounded-md border border-border-input bg-bg px-1 text-text" aria-label="Separator"><option value=",">comma</option><option value=";">semicolon</option><option value="tab">tab</option><option value="|">bar</option></select></label>
            <label class="flex items-center gap-1.5"><input v-model="imp.header" type="checkbox">First row is the header</label>
          </div>
          <table class="w-full table-fixed border-collapse text-xs" aria-label="Column mapping">
            <thead><tr class="text-left text-muted"><th class="h-7 w-28 font-medium">File</th><th class="w-6" /><th class="font-medium">Column</th></tr></thead>
            <tbody>
              <tr v-for="(f, i) in imp.preview.file_columns" :key="i" class="border-t border-surface-raised">
                <td class="h-8 truncate font-mono" :title="f">{{ f }}</td><td class="text-muted">→</td>
                <td><select v-model="imp.mapping[i]" class="h-7 w-full rounded-md border border-border-input bg-bg px-1 font-mono" :aria-label="`Column for ${f}`"><option :value="null">skip</option><option v-for="c in imp.preview.table_columns.filter(c => !c.generated)" :key="c.name" :value="c.name">{{ c.name }} · {{ c.type_name }}</option></select></td>
              </tr>
            </tbody>
          </table>
          <p class="text-xs text-muted">Runs in one transaction. Nothing is kept unless every row goes in. Empty values load as NULL (an empty text and NULL are not told apart).</p>
        </template>
        <p v-else class="text-xs text-muted">Pick a CSV file. Its columns are matched to the table's by name, and you check them before anything is loaded.</p>
        <p v-if="imp.done" role="status" class="text-xs text-success" data-part="done">{{ imp.done }}</p>
        <p v-if="imp.error" role="alert" class="text-xs text-danger" data-part="error">{{ imp.error }}</p>
        <div class="mt-auto flex justify-end gap-2"><Button variant="secondary" :disabled="!target" @click="chooseFile">Choose file…</Button><Button :disabled="!imp.preview || imp.mapping.every(m => !m)" @click="reviewImport">Review import</Button></div>
      </section>

      <section :class="panel" aria-label="Backup">
        <h2 class="text-base font-semibold">Back up {{ db?.name ?? 'this database' }}</h2>
        <p class="text-xs text-muted">Runs the dump tool {{ bk.info ? `on ${bk.info.runs_on}` : 'on the host' }}, so the dump is made next to the data and the tool's version matches the server. The password is never on a command line.</p>
        <dl class="grid grid-cols-[110px_minmax(0,1fr)] gap-y-1 rounded-md border border-border bg-bg p-2.5 text-xs">
          <dt class="text-muted">Tool</dt><dd class="font-mono" data-part="tool">{{ bk.info ? (bk.info.version ?? bk.info.tool) : '…' }}</dd>
          <dt class="text-muted">Format</dt><dd>{{ store.isMy ? 'SQL script (restorable with mysql)' : 'Custom (compressed, restorable)' }}</dd>
        </dl>
        <p v-if="bk.info?.problem" role="alert" class="text-xs text-danger" data-part="problem">{{ bk.info.problem }}</p>
        <p v-if="bk.running" role="status" class="text-xs text-muted" data-part="progress">{{ bk.kind === 'restore' ? 'Restoring' : 'Dumping' }}… {{ mb(bk.bytes) }} {{ bk.kind === 'restore' ? 'sent' : 'copied to this computer as it goes' }}</p>
        <p v-if="bk.done" role="status" class="text-xs text-success" data-part="done">{{ bk.done }}</p>
        <p v-if="bk.error" role="alert" class="text-xs text-danger" data-part="error">{{ bk.error }}</p>
        <p class="text-xs text-muted">Restore works the other way ({{ store.isMy ? 'the mysql client' : 'pg_restore' }}) and always asks for the target's name first.</p>
        <div class="mt-auto flex justify-end gap-2"><Button v-if="bk.running" variant="secondary" @click="store.stopTransfer(bk.id)">Stop</Button><Button variant="secondary" :disabled="bk.running || !!bk.info?.problem" @click="startRestore">Restore from file…</Button><Button :disabled="bk.running || !!bk.info?.problem" @click="backup">Back up…</Button></div>
      </section>
    </div>

    <AlertDialog v-model:open="importOpen">
      <AlertDialogContent v-if="imp.pending" class="gap-3 sm:max-w-[520px]" :data-testid="tid('import-confirm')">
        <AlertDialogHeader>
          <AlertDialogTitle>Load {{ fmt(imp.pending.rows) }} rows into {{ pick }} on {{ imp.pending.db_name }}?</AlertDialogTitle>
          <AlertDialogDescription>One transaction: if the server refuses any row, nothing is kept. <span v-if="imp.pending.environment === 'production'" class="text-warning">This is a production database.</span></AlertDialogDescription>
        </AlertDialogHeader>
        <pre class="whitespace-pre-wrap break-all rounded-md border border-border bg-bg p-2 font-mono text-xs" aria-label="Statement">{{ imp.pending.statement }}</pre>
        <AlertDialogFooter><AlertDialogCancel>Cancel</AlertDialogCancel><Button :loading="imp.busy" @click="commitImport">Load rows</Button></AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>

    <AlertDialog v-model:open="restoreOpen">
      <AlertDialogContent v-if="bk.restoring" class="gap-3 border-danger sm:max-w-[520px]" :data-testid="tid('restore-confirm')">
        <AlertDialogHeader>
          <AlertDialogTitle>Restore {{ bk.restoring.file_name }} into {{ bk.restoring.db_name }}?</AlertDialogTitle>
          <AlertDialogDescription>This replaces objects in <b>{{ bk.restoring.db_name }}</b> ({{ bk.restoring.environment === 'none' ? 'no environment set' : bk.restoring.environment }}) with the dump's. {{ mb(bk.restoring.file_bytes) }} are sent to {{ bk.restoring.runs_on }}. It can't be undone.</AlertDialogDescription>
        </AlertDialogHeader>
        <label class="flex flex-col gap-1 text-xs text-muted">Type {{ bk.restoring.db_name }} to confirm<Input v-model="bk.typed" class="font-mono text-xs text-text" autocomplete="off" spellcheck="false" /></label>
        <AlertDialogFooter><AlertDialogCancel>Cancel</AlertDialogCancel><Button :class="buttonVariants({ variant: 'destructive' })" :disabled="bk.typed !== bk.restoring.db_name" @click="doRestore">Restore</Button></AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
