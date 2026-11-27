<script setup lang="ts">
// Backup check (D17, E7): restore a backup into a scratch database `dalan_check_<id>` on a server that isn't
// production, count its tables (or collections) against this database, and drop it again. The file is the backup
// just made, one on this computer, or one in a folder on a host. Rust reads it back first; a file that doesn't read
// back isn't restored.
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import type { CheckFile } from '~/types/bindings'
import type { HostPick } from './HostFolderPicker.vue'

const props = defineProps<{ tabKey: string, fileId: string }>()
const open = defineModel<boolean>('open', { required: true })
const checks = useChecksStore()
const sessions = useSessionsStore()
const databases = useDatabasesStore()

const me = computed(() => sessions.sessions.find(s => s.key === props.tabKey))
const myDb = computed(() => databases.rows.find(d => d.id === me.value?.dbId))
const family = (e: string | undefined) => (e === 'mariadb' ? 'mysql' : e)
const connOf = (key: string) => (sessions.sessions.find(s => s.key === key)?.kind === 'mongo' ? useMongoStore(key).connId : useSqlStore(key).connId)
const readyOf = (key: string) => (sessions.sessions.find(s => s.key === key)?.kind === 'mongo' ? useMongoStore(key).status : useSqlStore(key).status) === 'ready'
// Servers to restore on: open tabs of the same engine that aren't production (this one too, when it isn't).
const servers = computed(() => sessions.sessions
  .filter(s => (s.kind === 'sql' || s.kind === 'mongo') && s.dbId)
  .map((s) => {
    const db = databases.rows.find(d => d.id === s.dbId)
    return { key: s.key, name: db?.name ?? s.label, engine: db?.engine, environment: db?.environment ?? 'none', ready: readyOf(s.key) }
  })
  .filter(s => family(s.engine) === family(myDb.value?.engine) && s.environment !== 'production'))
const server = ref('')
const source = ref<'last' | 'file' | 'host'>('last')
const hostPick = ref(false)
watch(open, (v) => {
  if (!v) return
  checks.reset()
  source.value = props.fileId ? 'last' : 'file'
  if (!servers.value.some(s => s.key === server.value)) server.value = servers.value[0]?.key ?? ''
})

async function run(file: CheckFile) {
  await checks.verify(connOf(props.tabKey), connOf(server.value), file)
}
function check() {
  if (source.value === 'host') {
    hostPick.value = true
    return
  }
  const id = source.value === 'last' ? props.fileId : checks.picked?.id
  if (id) void run({ kind: 'picked', file_id: id })
}
// A host's file is checked while the picker's SFTP session is still open (it is fetched through it).
const onHost = (p: HostPick) => run({ kind: 'host', sftp_id: p.sftpId, path: p.path })
const r = computed(() => checks.report)
const fmt = (n: number | null | undefined) => (n == null ? '—' : Math.round(n).toLocaleString('en-US'))
const canRun = computed(() => !!server.value && servers.value.find(s => s.key === server.value)?.ready && (source.value === 'host' || (source.value === 'last' ? !!props.fileId : !!checks.picked)))
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-[620px]" :data-testid="tid('backup-check-dialog')">
      <DialogHeader>
        <DialogTitle>Check a backup of {{ myDb?.name }}</DialogTitle>
        <DialogDescription>It is restored into a scratch database on a server that isn't production, its rows are counted against {{ myDb?.name }} as it is now, and the scratch database is dropped again.</DialogDescription>
      </DialogHeader>
      <div class="flex flex-col gap-3 text-sm">
        <div role="radiogroup" aria-label="Backup to check" class="flex flex-col gap-1.5 text-xs">
          <label class="flex items-center gap-2"><input v-model="source" type="radio" name="check-file" value="last" :disabled="!fileId" class="accent-primary">The backup just made</label>
          <label class="flex items-center gap-2"><input v-model="source" type="radio" name="check-file" value="file" class="accent-primary">A file on this computer
            <Button v-if="source === 'file'" size="sm" variant="secondary" @click="checks.pick(connOf(tabKey))">Choose a file…</Button>
            <span v-if="source === 'file' && checks.picked" class="truncate font-mono">{{ checks.picked.name }}</span>
          </label>
          <label class="flex items-center gap-2"><input v-model="source" type="radio" name="check-file" value="host" class="accent-primary">A file in a folder on a host (chosen when you press Check)</label>
        </div>
        <label class="flex flex-col gap-1 text-xs">Restore it on
          <select v-model="server" class="h-8 rounded-md border border-border-input bg-bg px-2 text-sm" aria-label="Restore it on">
            <option v-if="!servers.length" value="" disabled>Open a tab of a server that isn't production</option>
            <option v-for="s in servers" :key="s.key" :value="s.key" :disabled="!s.ready">{{ s.name }}{{ s.ready ? '' : ' (connecting…)' }}</option>
          </select>
        </label>
        <p v-if="checks.busy" role="status" class="text-xs text-muted">Reading, restoring and counting… this takes as long as a restore.</p>
        <p v-if="checks.error" role="alert" class="text-xs text-danger">{{ checks.error }}</p>
        <template v-if="r">
          <p :role="r.read.ok && r.same ? 'status' : 'alert'" :class="['text-xs', r.read.ok && r.same ? 'text-success' : 'text-danger']" data-part="verdict">
            <template v-if="!r.read.ok">It doesn't read back ({{ r.read.problem }}). Nothing was restored.</template>
            <template v-else-if="r.same">Restored on {{ r.restored_on }}: every table has the same rows as {{ myDb?.name }} now.</template>
            <template v-else>Restored on {{ r.restored_on }}: some counts differ from {{ myDb?.name }} now (rows written since the backup count too).</template>
          </p>
          <p class="text-xs text-muted">{{ r.read.summary }} · {{ (r.seconds ?? 0).toFixed(1) }} s</p>
          <table v-if="r.rows.length" class="w-full text-left text-xs" aria-label="Rows per table">
            <thead><tr class="text-muted"><th class="py-1 font-normal">Table</th><th class="text-right font-normal">In the backup</th><th class="text-right font-normal">In {{ myDb?.name }} now</th></tr></thead>
            <tbody>
              <tr v-for="x in r.rows" :key="x.name" class="border-t border-border">
                <td class="py-1 font-mono">{{ x.name }}</td>
                <td class="text-right tabular-nums">{{ fmt(x.restored) }}</td>
                <td :class="['text-right tabular-nums', x.same ? '' : 'text-warning']">{{ x.source == null ? 'not there' : fmt(x.source) }}</td>
              </tr>
            </tbody>
          </table>
        </template>
      </div>
      <DialogFooter>
        <Button variant="secondary" @click="open = false">Close</Button>
        <Button :disabled="!canRun || checks.busy" :loading="checks.busy" :data-testid="tid('backup-check-run')" @click="check">Check</Button>
      </DialogFooter>
      <HostFolderPicker v-model:open="hostPick" mode="file" title="A backup on a host" :on-pick="onHost" />
    </DialogContent>
  </Dialog>
</template>
