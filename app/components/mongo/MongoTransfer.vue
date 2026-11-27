<script setup lang="ts">
// S43 for MongoDB: export a collection (one extended JSON document per line), and back up or restore the database
// with mongodump and mongorestore on the host. Rust writes and reads the files; the window shows progress. A restore
// is always typed.
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { BackupInfo_Serialize as BackupInfo, TransferProgress } from '~/types/bindings'

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
async function doExport() {
  const t = target.value
  if (!t || exp.running) return
  Object.assign(exp, { running: true, id: store.newTransferId(), rows: 0, done: '', error: '' })
  try {
    const r = await store.exportColl(exp.id, t.db, t.coll, exp.filter, (p: TransferProgress) => { exp.rows = p.rows ?? 0 })
    exp.done = r ? `Saved ${r.path} · ${fmt(r.rows)} documents · ${mb(r.bytes)}` : ''
  }
  catch (e) { exp.error = message(e, 'The export failed.') }
  exp.running = false
}

// ---- Backup and restore
const bk = reactive({ info: null as BackupInfo | null, running: false, id: '', bytes: 0, done: '', error: '' })
const restore = useRestoreStore(props.tabKey)
// Closing the tab ends what it started: nothing keeps running unseen.
onBeforeUnmount(() => { if (exp.running) void store.stopTransfer(exp.id); if (bk.running) void store.stopTransfer(bk.id) })
onMounted(async () => {
  try { bk.info = await store.backupCheck() }
  catch (e) { bk.error = message(e, 'Couldn\'t check the tool.') }
})
async function backup() {
  if (bk.running) return
  Object.assign(bk, { running: true, id: store.newTransferId(), bytes: 0, done: '', error: '' })
  try {
    const r = await store.backupStart(bk.id, (p: TransferProgress) => { bk.bytes = p.bytes ?? 0 })
    bk.done = r ? `Saved ${r.path} · ${mb(r.bytes ?? 0)}` : ''
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
        <p v-if="bk.error" role="alert" class="text-xs text-danger" data-part="error">{{ bk.error }}</p>
        <p class="text-xs text-muted">Restore takes any mongodump archive or folder, from Dalan or made elsewhere, into this database or a new one. You see what is inside before anything runs.</p>
        <div class="mt-auto flex justify-end gap-2"><Button v-if="bk.running" variant="secondary" @click="store.stopTransfer(bk.id)">Stop</Button><Button variant="secondary" :disabled="bk.running" :data-testid="tid('mongo-restore')" @click="restore.start(store.connId, db?.environment === 'production')">Restore…</Button><Button :disabled="bk.running || !!bk.info?.problem" :data-testid="tid('mongo-backup')" @click="backup">Back up…</Button></div>
      </section>
    </div>

    <RestoreDialog v-if="db" :tab-key="tabKey" :db-name="db.name" :database="db.default_database" :environment="db.environment" :engine="db.engine" />
  </div>
</template>
