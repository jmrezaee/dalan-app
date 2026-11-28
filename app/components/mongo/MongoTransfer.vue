<script setup lang="ts">
// S43 for MongoDB: export a collection (one extended JSON document per line), and back up or restore the database
// with mongodump and mongorestore on the host. Rust writes and reads the files; the window shows progress. A restore
// is always typed.
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { MaskKind, BackupInfo_Serialize as BackupInfo, TransferProgress, HostFolder } from '~/types/bindings'

const props = defineProps<{ tabKey: string }>()
const store = useMongoStore(props.tabKey)
const databases = useDatabasesStore()
const db = computed(() => databases.rows.find(d => d.id === store.dbId) ?? null)
const message = (e: unknown, fallback: string) => (e instanceof Error ? e.message : (e as { message?: string })?.message ?? fallback)
const fmt = (n: number | null) => Math.round(n ?? 0).toLocaleString('en-US')
const mb = (raw: number | null) => { const n = raw ?? 0; return n >= 1 << 20 ? `${(n / (1 << 20)).toFixed(1)} MB` : `${Math.round(n / 1024)} kB` }
const panel = 'flex flex-col gap-3 rounded-lg border border-border bg-surface p-4 text-sm'

// ---- Export
const colls = computed(() => store.dbs.flatMap(d => (d.colls ?? []).filter(c => c.kind === 'collection').map(c => ({ id: `${d.name}.${c.name}`, db: d.name, coll: c.name }))))
const pick = ref('')
// The choice follows the list: a collection that is no longer there is dropped.
watch(colls, (c) => { if (!c.some(x => x.id === pick.value)) pick.value = c.length ? (store.current && c.some(x => x.id === `${store.current!.db}.${store.current!.coll}`) ? `${store.current.db}.${store.current.coll}` : c[0]!.id) : '' }, { immediate: true })
const target = computed(() => colls.value.find(c => c.id === pick.value) ?? null)
const exp = reactive({ filter: '', running: false, id: '', rows: 0, done: '', error: '' })
// Masking (E5): rules for top-level fields, applied in Rust before anything is written.
const masks = reactive<{ column: string, kind: MaskKind, value: string }[]>([])
async function doExport() {
  const t = target.value
  if (!t || exp.running) return
  Object.assign(exp, { running: true, id: store.newTransferId(), rows: 0, done: '', error: '' })
  try {
    const r = await store.exportColl(exp.id, t.db, t.coll, exp.filter, (p: TransferProgress) => { exp.rows = p.rows ?? 0 }, masks.filter(m => m.column.trim()).map(m => ({ column: m.column.trim(), kind: m.kind, value: m.value.trim() || null })))
    exp.done = r ? `Saved ${r.path} · ${fmt(r.rows)} documents · ${mb(r.bytes)}` : ''
  }
  catch (e) { exp.error = message(e, 'The export failed.') }
  exp.running = false
}

// ---- Backup and restore
const bk = reactive({ info: null as BackupInfo | null, running: false, id: '', bytes: 0, done: '', error: '', checked: '', checkOk: true, fileId: '' })
// D17 (E7): the backup check (restore into a scratch database and count rows).
const checkOpen = ref(false)
const restore = useRestoreStore(props.tabKey)
// Closing the tab ends what it started: nothing keeps running unseen.
onBeforeUnmount(() => { if (exp.running) void store.stopTransfer(exp.id); if (bk.running) void store.stopTransfer(bk.id) })
onMounted(async () => {
  try { bk.info = await store.backupCheck() }
  catch (e) { bk.error = message(e, 'Couldn\'t check the tool.') }
})
// A saved backup job (D14): run into its folder when Jobs asks, or save this backup as one.
const job = reactive({ naming: false, name: '', saved: '' })
const hostPick = ref(false)
watch(() => backupRequests.value.get(props.tabKey), async (id) => {
  if (!id || bk.running) return
  backupRequests.value.delete(props.tabKey)
  Object.assign(bk, { running: true, id: store.newTransferId(), bytes: 0, done: '', error: '' })
  try {
    const r = await store.backupJob(id, bk.id, (p: TransferProgress) => { bk.bytes = p.bytes ?? 0 })
    bk.done = `Saved ${r.path} · ${mb(r.bytes ?? 0)}`
  }
  catch (e) { bk.error = message(e, 'The backup failed.') }
  bk.running = false
}, { immediate: true })
/** Saves the backup as a job: into a folder chosen here, or (D15) a folder on a host picked over SFTP. */
async function saveBackupJob(host: HostFolder | null = null) {
  bk.error = ''
  try {
    const row = await store.saveBackupJob(job.name, host)
    if (row) {
      job.saved = `Saved as the job “${row.name}”. Run it from Jobs.`
      job.naming = false
      void useJobsStore().refresh()
    }
  }
  catch (e) { bk.error = message(e, 'Couldn\'t save the job.') }
}
async function backup() {
  if (bk.running) return
  Object.assign(bk, { running: true, id: store.newTransferId(), bytes: 0, done: '', error: '' })
  try {
    const r = await store.backupStart(bk.id, (p: TransferProgress) => { bk.bytes = p.bytes ?? 0 })
    bk.done = r ? `Saved ${r.path} · ${mb(r.bytes ?? 0)}` : ''
    // D17: every backup is read back at once.
    bk.checked = r?.check ? (r.check.ok ? `Read back: ${r.check.summary}.` : `It doesn't read back: ${r.check.problem ?? r.check.summary}`) : ''
    bk.checkOk = r?.check?.ok ?? true
    bk.fileId = r?.file_id ?? ''
  }
  catch (e) { bk.error = message(e, 'The backup failed.') }
  bk.running = false
}
</script>

<template>
  <div class="min-h-0 flex-1 overflow-auto p-4" :data-testid="tid('mongo-transfer')">
    <div class="grid max-w-[980px] grid-cols-1 gap-4 lg:grid-cols-2">
      <section :class="panel" aria-label="Export">
        <h2 class="text-base font-semibold">Export a collection</h2>
        <p class="text-xs text-muted">One extended JSON document per line, so ObjectIds, dates and Long numbers keep their types (the format mongoimport reads).</p>
        <p v-if="!colls.length" class="text-xs text-muted" data-part="no-collections">Open a database in the list on the left first: only the collections you have seen there can be picked.</p>
        <label class="flex flex-col gap-1 text-xs text-muted">Collection
          <select v-model="pick" class="h-8 rounded-md border border-border-input bg-bg px-2 text-sm text-text" aria-label="Collection to export"><option v-for="c in colls" :key="c.id" :value="c.id">{{ c.id }}</option></select>
        </label>
        <label class="flex flex-col gap-1 text-xs text-muted">Filter (optional)
          <Input v-model="exp.filter" class="h-8 font-mono text-xs text-text" placeholder="{ &quot;status&quot;: &quot;open&quot; }" autocomplete="off" spellcheck="false" aria-label="Export filter" />
        </label>
        <details class="text-xs" :data-testid="tid('export-masks')"><summary class="cursor-default text-muted">Mask fields{{ masks.length ? ` · ${masks.length}` : '' }}</summary>
          <div v-for="(m, i) in masks" :key="i" class="mt-1 flex items-center gap-1">
            <Input v-model="m.column" class="h-7 w-28 font-mono text-xs" placeholder="field" :aria-label="`Masked field ${i + 1}`" />
            <MaskKindSelect v-model="m.kind" :label="`Masking for field ${i + 1}`" />
            <Input v-if="['fixed', 'truncate', 'date_shift'].includes(m.kind)" v-model="m.value" class="h-7 w-20 text-xs" :placeholder="m.kind === 'fixed' ? 'value' : m.kind === 'truncate' ? 'chars' : '± days'" :aria-label="`Value for field ${i + 1}`" />
            <Button size="sm" variant="ghost" :aria-label="`Remove rule ${i + 1}`" @click="masks.splice(i, 1)">×</Button>
          </div>
          <Button size="sm" variant="ghost" class="mt-1" @click="masks.push({ column: '', kind: 'hash', value: '' })">Add a rule</Button>
          <p class="mt-1 text-muted">Top-level fields. Shuffle isn't available for MongoDB; it leaves the field as it is.</p>
        </details>
        <p v-if="exp.running" role="status" class="text-xs text-muted" data-part="progress">Writing… {{ fmt(exp.rows) }} documents</p>
        <p v-if="exp.done" role="status" class="text-xs text-success" data-part="done">{{ exp.done }}</p>
        <p v-if="exp.error" role="alert" class="text-xs text-danger" data-part="error">{{ exp.error }}</p>
        <div class="mt-auto flex justify-end gap-2"><Button v-if="exp.running" variant="secondary" @click="store.stopTransfer(exp.id)">Stop</Button><Button :disabled="exp.running || !target" :data-testid="tid('mongo-export')" @click="doExport">Export…</Button></div>
      </section>

      <section :class="panel" aria-label="Backup">
        <h2 class="text-base font-semibold">Back up {{ db?.name ?? 'this database' }}</h2>
        <p class="text-xs text-muted">Runs mongodump {{ bk.info ? `on ${bk.info.runs_on}` : 'on the host' }}, so the dump is made next to the data and the tool's version matches the server. The password is never on a command line.</p>
        <dl class="grid grid-cols-[110px_minmax(0,1fr)] gap-y-1 rounded-md border border-border bg-bg p-2.5 text-xs">
          <dt class="text-muted">Tool</dt><dd class="font-mono" data-part="tool">{{ bk.info ? (bk.info.version ?? bk.info.tool) : '…' }}</dd>
          <dt class="text-muted">Format</dt><dd>Archive, compressed (restorable with mongorestore)</dd>
        </dl>
        <p v-if="bk.info?.problem" role="alert" class="text-xs text-danger" data-part="problem">{{ bk.info.problem }}</p>
        <p v-if="bk.running" role="status" class="text-xs text-muted" data-part="progress">Dumping… {{ mb(bk.bytes) }} copied to this computer as it goes</p>
        <p v-if="bk.done" role="status" class="text-xs text-success" data-part="done">{{ bk.done }}</p>
        <p v-if="bk.checked" :role="bk.checkOk ? 'status' : 'alert'" :class="['text-xs', bk.checkOk ? 'text-muted' : 'text-danger']" data-part="read-check">{{ bk.checked }}</p>
        <BackupCheckDialog v-model:open="checkOpen" :tab-key="tabKey" :file-id="bk.fileId" />
        <p v-if="bk.error" role="alert" class="text-xs text-danger" data-part="error">{{ bk.error }}</p>
        <p class="text-xs text-muted">Restore takes any mongodump archive or folder, from Dalan or made elsewhere, into this database or a new one. You see what is inside before anything runs.</p>
                <div v-if="job.naming" class="flex items-center gap-2"><Input v-model="job.name" class="h-7 text-xs" placeholder="Nightly backup" aria-label="Job name" /><Button size="sm" :disabled="!job.name.trim()" :data-testid="tid('backup-save-job')" @click="saveBackupJob()">Choose folder and save</Button><Button size="sm" variant="secondary" :disabled="!job.name.trim()" :data-testid="tid('backup-save-job-host')" @click="hostPick = true">On a host…</Button></div>
        <HostFolderPicker v-model:open="hostPick" mode="folder" title="Folder for the backups" :on-pick="(p) => saveBackupJob({ host_id: p.hostId, dir: p.path })" />
        <p v-if="job.saved" role="status" class="text-xs text-success">{{ job.saved }}</p>
<div class="mt-auto flex flex-wrap justify-end gap-2"><Button v-if="bk.running" variant="secondary" @click="store.stopTransfer(bk.id)">Stop</Button><Button variant="ghost" :disabled="bk.running || !!bk.info?.problem" @click="job.naming = !job.naming">Save as a job…</Button><Button variant="ghost" :disabled="bk.running" :data-testid="tid('backup-check')" @click="checkOpen = true">Check a backup…</Button><Button variant="secondary" :disabled="bk.running" :data-testid="tid('mongo-restore')" @click="restore.start(store.connId, db?.environment === 'production')">Restore…</Button><Button :disabled="bk.running || !!bk.info?.problem" :data-testid="tid('mongo-backup')" @click="backup">Back up…</Button></div>
      </section>
    </div>
    <!-- D17 (E6): many collections at once, and MongoDB import. -->
    <div class="mt-4 grid max-w-[980px] grid-cols-1 gap-4 lg:grid-cols-2">
      <BulkExportPanel :tab-key="tabKey" :conn-id="store.connId" :objects="colls.map(c => ({ schema: c.db, name: c.coll }))" :mongo="true" />
      <MongoImportPanel :tab-key="tabKey" :conn-id="store.connId" :dbs="store.dbs.map(d => d.name)" />
    </div>

    <RestoreDialog v-if="db" :tab-key="tabKey" :db-name="db.name" :database="db.default_database" :environment="db.environment" :engine="db.engine" />
  </div>
</template>
