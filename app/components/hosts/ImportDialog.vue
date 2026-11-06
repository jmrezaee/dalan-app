<script setup lang="ts">
// S25 / F10: import hosts from an OpenSSH config or a CSV. Pick a source, review the list, then import.
// The webview only chooses a path; Rust reads the file and parses it.
import { FolderOpen, TriangleAlert } from '@lucide/vue'
import type { ImportKind, ImportRow } from '~/types/bindings'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Checkbox } from '@/components/ui/checkbox'
import { Label } from '@/components/ui/label'
import { Badge } from '@/components/ui/badge'
import { toast } from 'vue-sonner'

const open = defineModel<boolean>('open', { required: true })
const hosts = useHostsStore()

type Kind = ImportKind
const kind = ref<Kind>('ssh_config')
const rows = ref<ImportRow[]>([])
const fileWarnings = ref<string[]>([])
const picked = ref<Set<number>>(new Set())
const groupId = ref<string | null>(null)
const previewed = ref(false)
const error = ref('')
const busy = ref(false)

watch(open, (o) => {
  if (!o) return
  kind.value = 'ssh_config'
  rows.value = []
  fileWarnings.value = []
  picked.value = new Set()
  groupId.value = null
  previewed.value = false
  filter.value = ''
  error.value = ''
})
watch(kind, () => { previewed.value = false; error.value = '' })

async function load(path: string | null) {
  busy.value = true
  error.value = ''
  try {
    const p = await hosts.importPreview(kind.value, path)
    rows.value = p.rows
    fileWarnings.value = p.warnings
    // Hosts you already have start unticked.
    picked.value = new Set(p.rows.flatMap((r, i) => (r.duplicate ? [] : [i])))
    previewed.value = true
  }
  catch (e) {
    error.value = e instanceof Error ? e.message : 'Couldn\'t read that file.'
  }
  busy.value = false
}

async function choose() {
  try {
    const p = await hosts.pickFile()
    if (p) await load(p)
  }
  catch {
    error.value = 'Couldn\'t open the file picker.'
  }
}

// A filter box and, when the file has folders or groups, one tick box per group.
const filter = ref('')
const shown = computed(() => {
  const q = filter.value.trim().toLowerCase()
  return rows.value.flatMap((r, i) => (!q || `${r.label} ${r.address} ${r.username ?? ''} ${r.group ?? ''}`.toLowerCase().includes(q) ? [i] : []))
})
const shownRows = computed(() => shown.value.map(i => ({ i, r: rows.value[i]! })))
const sourceGroups = computed(() => [...new Set(rows.value.map(r => r.group).filter((g): g is string => !!g))])
const groupIndexes = (g: string) => rows.value.flatMap((r, i) => (r.group === g ? [i] : []))
const groupOn = (g: string) => groupIndexes(g).every(i => picked.value.has(i))
function toggleGroup(g: string) {
  const on = groupOn(g)
  const s = new Set(picked.value)
  for (const i of groupIndexes(g)) { if (on) s.delete(i); else s.add(i) }
  picked.value = s
}
const allOn = computed(() => shown.value.length > 0 && shown.value.every(i => picked.value.has(i)))
function toggle(i: number) {
  const s = new Set(picked.value)
  if (!s.delete(i)) s.add(i)
  picked.value = s
}
function toggleAll() {
  // Acts on the rows the filter shows.
  const s = new Set(picked.value)
  for (const i of shown.value) { if (allOn.value) s.delete(i); else s.add(i) }
  picked.value = s
}

async function submit() {
  if (busy.value || !picked.value.size) return
  busy.value = true
  try {
    const chosen = rows.value.filter((_, i) => picked.value.has(i))
    const r = await hosts.importApply(chosen, groupId.value)
    await hosts.refresh()
    const missing = r.jumps_missing ? ` ${r.jumps_missing} jump ${r.jumps_missing === 1 ? 'host was' : 'hosts were'} not found, so the route was left out. Tick them too, or add the route in the host editor.` : ''
    toast((r.skipped ? `Imported ${r.created} hosts. ${r.skipped} couldn't be added.` : `Imported ${r.created} ${r.created === 1 ? 'host' : 'hosts'}.`) + missing)
    open.value = false
  }
  catch (e) {
    error.value = e instanceof Error ? e.message : 'Import failed.'
  }
  busy.value = false
}
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-[640px]">
      <DialogHeader>
        <DialogTitle>Import hosts</DialogTitle>
        <DialogDescription>Bring over servers from your SSH config, a spreadsheet, PuTTY or MobaXterm. You'll review the list first.</DialogDescription>
      </DialogHeader>

      <div role="radiogroup" aria-label="Source" class="flex w-fit overflow-hidden rounded-md border border-border text-sm">
        <button
          v-for="o in [{ v: 'ssh_config', t: 'SSH config' }, { v: 'csv', t: 'CSV file' }, { v: 'putty', t: 'PuTTY' }, { v: 'mobaxterm', t: 'MobaXterm' }] as const"
          :key="o.v"
          type="button"
          role="radio"
          :aria-checked="kind === o.v"
          class="px-3 py-1.5 transition-colors"
          :class="kind === o.v ? 'bg-accent text-on-accent' : 'hover:bg-surface-hover'"
          @click="kind = o.v"
        >
          {{ o.t }}
        </button>
      </div>

      <div v-if="!previewed" class="flex flex-col gap-3">
        <template v-if="kind === 'ssh_config'">
          <p class="text-sm text-muted">Reads Host entries with their HostName, User, Port and ProxyJump. Wildcard entries only supply defaults.</p>
          <div class="flex gap-2">
            <Button :loading="busy" @click="load(null)">Read ~/.ssh/config</Button>
            <Button variant="secondary" :disabled="busy" @click="choose"><FolderOpen />Choose file…</Button>
          </div>
        </template>
        <template v-else-if="kind === 'putty'">
          <p class="text-sm text-muted">Choose a <span class="font-mono text-xs">.reg</span> export of your PuTTY sessions (in regedit, export <span class="font-mono text-xs">HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions</span>), or a session file from <span class="font-mono text-xs">~/.putty/sessions</span>. Only SSH sessions come across; key files and passwords don't.</p>
          <div><Button :loading="busy" @click="choose"><FolderOpen />Choose PuTTY file…</Button></div>
        </template>
        <template v-else-if="kind === 'mobaxterm'">
          <p class="text-sm text-muted">Choose a MobaXterm export (Settings, Configuration, then Export, giving a <span class="font-mono text-xs">.mobaconf</span> file). SSH bookmarks come across with their folders; other session types and passwords don't.</p>
          <div><Button :loading="busy" @click="choose"><FolderOpen />Choose MobaXterm file…</Button></div>
        </template>
        <template v-else>
          <p class="text-sm text-muted">
            The first row names the columns. <span class="font-mono text-xs">address</span> is required; <span class="font-mono text-xs">label, port, username, group, tags</span> are optional. Separate tags with <span class="font-mono text-xs">;</span>.
          </p>
          <div><Button :loading="busy" @click="choose"><FolderOpen />Choose CSV file…</Button></div>
        </template>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
      </div>

      <div v-else class="flex flex-col gap-3">
        <ul v-if="fileWarnings.length" class="flex flex-col gap-1 rounded-md border border-border bg-surface-raised p-2.5 text-xs text-muted">
          <li v-for="w in fileWarnings" :key="w" class="flex gap-2"><TriangleAlert class="mt-0.5 size-3.5 shrink-0 text-warning" />{{ w }}</li>
        </ul>

        <div v-if="rows.length > 1" class="flex flex-wrap items-center gap-3">
          <SearchInput v-model="filter" placeholder="Filter the list" :result-count="shown.length" />
          <label v-for="g in sourceGroups" :key="g" class="flex items-center gap-1.5 text-xs text-muted"><Checkbox :model-value="groupOn(g)" :aria-label="`Select the ${g} group`" @update:model-value="toggleGroup(g)" />{{ g }}</label>
        </div>
        <div v-if="rows.length" class="max-h-[320px] overflow-auto rounded-md border border-border">
          <div class="sticky top-0 flex items-center gap-3 border-b border-border bg-surface px-3 py-2 text-xs text-muted">
            <Checkbox :model-value="allOn" aria-label="Select all" @update:model-value="toggleAll" />
            {{ picked.size }} of {{ rows.length }} selected
          </div>
          <p v-if="!shown.length" class="px-3 py-4 text-sm text-muted">No rows match.</p>
          <label v-for="{ i, r } in shownRows" :key="i" class="flex cursor-pointer items-start gap-3 border-b border-border px-3 py-2 last:border-b-0 hover:bg-surface-hover">
            <Checkbox class="mt-1" :model-value="picked.has(i)" :aria-label="`Import ${r.label}`" @update:model-value="toggle(i)" />
            <span class="min-w-0 flex-1">
              <span class="flex items-center gap-2 text-sm">
                <span class="truncate font-medium">{{ r.label }}</span>
                <Badge v-if="r.duplicate" variant="secondary">Already added</Badge>
                <Badge v-if="r.group" variant="secondary">{{ r.group }}</Badge>
              </span>
              <span class="block truncate font-mono text-xs text-muted">{{ r.username ? `${r.username}@` : '' }}{{ r.address }}{{ r.port ? `:${r.port}` : '' }}<template v-if="r.jump.length"> via {{ r.jump.join(' → ') }}</template></span>
              <span v-for="w in r.warnings" :key="w" class="mt-0.5 block text-xs text-warning">{{ w }}</span>
            </span>
          </label>
        </div>

        <div v-if="rows.length" class="flex flex-col gap-1.5">
          <Label for="imp-group">Add to group</Label>
          <GroupSelect id="imp-group" v-model="groupId" :options="hosts.groupOptions()" />
          <p class="text-xs text-muted">Rows with their own group keep it.</p>
        </div>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <div class="flex justify-end gap-2">
          <Button variant="secondary" @click="previewed = false">Back</Button>
          <Button :loading="busy" :disabled="!picked.size" @click="submit">Import {{ picked.size }} {{ picked.size === 1 ? 'host' : 'hosts' }}</Button>
        </div>
      </div>
    </DialogContent>
  </Dialog>
</template>
