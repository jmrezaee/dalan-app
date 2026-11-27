<script setup lang="ts">
// D15 (E8): pick a folder (a backup job's destination) or a dump file (a restore) on a saved host, over SFTP. It uses
// an SFTP store of its own, so the host key and password questions are the usual ones, answered inline. The SFTP
// session stays open until `onPick` has finished (a restore fetches the file through it), then closes.
import { ArrowUp, File, Folder, Server } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'

export interface HostPick { hostId: string, sftpId: string, path: string }

const props = defineProps<{ mode: 'folder' | 'file', title: string, onPick: (p: HostPick) => Promise<void> }>()
const open = defineModel<boolean>('open', { required: true })

const hosts = useHostsStore()
const sftp = useSftpStore('host-picker')
const pane = computed(() => sftp.panes[1])
const hostId = ref('')
const chosen = ref('')
const typed = ref('')
const busy = ref(false)
const error = ref('')
const connected = computed(() => isRemote(pane.value) && pane.value.hostId === hostId.value)
const label = computed(() => hosts.hosts.find(h => h.id === hostId.value)?.label ?? '')

watch(open, (v) => {
  if (v) {
    if (!hosts.hosts.length) void hosts.refresh()
    error.value = ''
    chosen.value = ''
  }
  else sftp.clear()
})
watch(() => pane.value.path, (p) => { typed.value = p; chosen.value = '' })

async function connect() {
  const h = hosts.hosts.find(x => x.id === hostId.value)
  if (h) await sftp.connect(1, h)
}
const entries = computed(() => sftp.sorted(pane.value).filter(e => e.kind === 'dir' || props.mode === 'file'))
const target = computed(() => (props.mode === 'folder' ? pane.value.path : chosen.value ? joinPath(pane.value, chosen.value) : ''))

async function use() {
  if (!target.value) return
  busy.value = true
  error.value = ''
  try {
    await props.onPick({ hostId: hostId.value, sftpId: pane.value.endpoint, path: target.value })
    open.value = false
  }
  catch (e) { error.value = (e as { message?: string })?.message ?? 'That didn\'t work.' }
  busy.value = false
}
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-[560px]" :data-testid="tid('host-picker')">
      <DialogHeader>
        <DialogTitle>{{ title }}</DialogTitle>
        <DialogDescription>On a saved host, over SFTP. Saved jobs connect again with no one to ask, so trust the host's key and save its password or key.</DialogDescription>
      </DialogHeader>
      <div class="flex items-center gap-2">
        <Server class="size-4 shrink-0 text-muted" aria-hidden="true" />
        <select v-model="hostId" class="h-8 min-w-0 flex-1 rounded-md border border-border-input bg-bg px-2 text-sm" aria-label="Host">
          <option value="" disabled>Choose a host</option>
          <option v-for="h in hosts.hosts" :key="h.id" :value="h.id">{{ h.label }}</option>
        </select>
        <Button size="sm" variant="secondary" :disabled="!hostId || connected" :loading="pane.connecting" @click="connect">Connect</Button>
      </div>
      <div v-if="sftp.prompt" class="rounded-md border border-border bg-surface p-3">
        <PromptPanel :kind="sftp.prompt.kind" :host="label" @reply="sftp.answer" />
      </div>
      <p v-if="pane.error" role="alert" class="text-sm text-danger">{{ pane.error }}</p>
      <template v-if="connected">
        <div class="flex items-center gap-2">
          <Button size="sm" variant="ghost" aria-label="Up one folder" @click="sftp.up(1)"><ArrowUp class="size-4" aria-hidden="true" /></Button>
          <Input v-model="typed" class="h-8 font-mono text-xs" aria-label="Folder on the host" @keydown.enter.prevent="sftp.go(1, typed)" />
        </div>
        <ul class="m-0 h-56 list-none overflow-auto rounded-md border border-border p-1" :aria-label="`Contents of ${pane.path}`">
          <li v-if="!entries.length && !pane.loading" class="p-2 text-xs text-muted">{{ mode === 'folder' ? 'No folders in here.' : 'Nothing in here.' }}</li>
          <li v-for="e in entries" :key="e.name">
            <button
              type="button"
              :class="['flex w-full items-center gap-2 rounded px-2 py-1 text-left text-sm hover:bg-surface-hover', chosen === e.name && 'bg-surface-hover text-text']"
              :aria-pressed="e.kind === 'dir' ? undefined : chosen === e.name"
              @click="e.kind === 'dir' ? sftp.open(1, e) : (chosen = e.name)"
            >
              <component :is="e.kind === 'dir' ? Folder : File" class="size-4 shrink-0 text-muted" aria-hidden="true" />
              <span class="truncate">{{ e.name }}</span>
            </button>
          </li>
        </ul>
      </template>
      <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
      <DialogFooter>
        <Button variant="secondary" @click="open = false">Cancel</Button>
        <Button :disabled="!connected || !target" :loading="busy" :data-testid="tid('host-picker-use')" @click="use">{{ mode === 'folder' ? 'Use this folder' : 'Use this file' }}</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
