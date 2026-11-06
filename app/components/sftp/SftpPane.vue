<script setup lang="ts">
// One side of S21: where it is (this computer or a server), a path bar, and the files.
import { ArrowLeftRight, ChevronDown, Eye, EyeOff, Ellipsis, FolderPlus, KeyRound, Laptop, Lock, ServerOff, Loader, Pencil, RefreshCw, Server, Trash2 } from '@lucide/vue'
import type { FsEntry } from '~/types/bindings'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import type { MenuItem } from '@/components/common/ItemMenuList.vue'
import { ContextMenu, ContextMenuContent, ContextMenuTrigger } from '@/components/ui/context-menu'
import { formatBytes } from '~/utils/format'
import { isRemote, parentPath } from '~/stores/sftp'
import { toast } from 'vue-sonner'

const { fixed: shortcutFixed } = useShortcutLabel()

const props = defineProps<{ index: number }>()
const emit = defineEmits<{ newFolder: [], rename: [name: string], remove: [names: string[]], chmod: [name: string] }>()
const sftp = useSftpStore()
const hosts = useHostsStore()

const pane = computed(() => sftp.panes[props.index]!)
const other = computed(() => sftp.panes[props.index === 0 ? 1 : 0]!)
const rows = computed(() => sftp.sorted(pane.value))
const remote = computed(() => isRemote(pane.value))
const picked = computed(() => rows.value.filter(e => pane.value.selected.includes(e.name)))
const foot = computed(() => {
  const sel = picked.value
  if (sel.length) return `${sel.length} selected · ${formatBytes(sel.reduce((n, e) => n + (e.kind === 'file' ? (e.size ?? 0) : 0), 0))}`
  return `${rows.value.length} ${rows.value.length === 1 ? 'item' : 'items'}`
})
/** Friendly copy for the two errors people actually hit; anything else shows the server's message. */
const problem = computed(() => {
  const p = pane.value
  if (p.errorCode === 'permission_denied') return { icon: Lock, title: 'No access', text: `You don't have permission to open ${p.path || 'that folder'}. Go up a folder, or ask whoever runs the server.` }
  if (p.errorCode === 'sftp_disabled') return { icon: ServerOff, title: `SFTP isn't enabled on ${p.label || 'this server'}`, text: 'The server doesn\'t offer file transfer. Its owner can turn it on with a Subsystem sftp line in sshd_config.' }
  return { icon: KeyRound, title: '', text: p.error }
})
const overDrop = ref(false)
const ctx = ref<FsEntry | null>(null)

function setSort(by: 'name' | 'size' | 'mtime') {
  const s = pane.value.sort
  if (s.by === by) s.desc = !s.desc
  else Object.assign(s, { by, desc: false })
}
const copyOver = () => sftp.copy(props.index, pane.value.selected)

// Keyboard: F2 renames, ⌘C / ⌘V (Ctrl elsewhere) copy a selection over to the other pane. Rows hold focus, so keys bubble up here.
function onKey(e: KeyboardEvent) {
  if ((e.target as HTMLElement).closest('input, textarea')) return
  if (e.key === 'F2' && picked.value.length === 1) {
    e.preventDefault()
    emit('rename', picked.value[0]!.name)
    return
  }
  if (!(e.metaKey || e.ctrlKey) || e.altKey) return
  const k = e.key.toLowerCase()
  if (k === 'c' && picked.value.length) {
    e.preventDefault()
    sftp.clipboard = { pane: props.index, names: picked.value.map(p => p.name) }
    toast(`Copied ${picked.value.length} ${picked.value.length === 1 ? 'item' : 'items'}. Press ${shortcutFixed('Mod+V')} in the other pane.`)
  }
  else if (k === 'v' && sftp.clipboard && sftp.clipboard.pane !== props.index && pane.value.endpoint) {
    e.preventDefault()
    sftp.copy(sftp.clipboard.pane, sftp.clipboard.names, props.index)
  }
}
const dirName = computed(() => (props.index === 0 ? 'right' : 'left'))

// Same rule as Rust: no extension for ".bashrc" or "Makefile".
const fileExt = (n: string) => (n.lastIndexOf('.') > 0 ? n.slice(n.lastIndexOf('.') + 1).toLowerCase() : '')

// The right-click menu and the visible "Actions" button share this list. It acts on the clicked row, or the selection.
const fileMenu = computed<MenuItem[]>(() => {
  const one = picked.value.length === 1 ? picked.value[0]! : null
  return [
    ...(ctx.value && ctx.value.kind === 'dir' ? [{ label: 'Open', onSelect: () => sftp.open(props.index, ctx.value!) }] : []),
    ...(picked.value.length && other.value.endpoint ? [{ label: `Copy to ${dirName.value} pane`, onSelect: copyOver }] : []),
    ...(one && remote.value && one.kind === 'file'
      ? [
          { label: 'Edit in default app', separator: true, onSelect: () => sftp.edit(props.index, one.name) },
          ...(sftp.openWith[fileExt(one.name)] ? [{ label: `Edit in ${sftp.openWith[fileExt(one.name)]}`, onSelect: () => sftp.edit(props.index, one.name, 'saved') }] : []),
          { label: 'Open with…', onSelect: () => sftp.edit(props.index, one.name, 'choose') },
        ]
      : []),
    ...(one ? [{ label: 'Rename', icon: Pencil, hint: 'F2', onSelect: () => emit('rename', one.name) }] : []),
    ...(one && remote.value ? [{ label: 'Permissions…', onSelect: () => emit('chmod', one.name) }] : []),
    ...(picked.value.length ? [{ label: 'Delete', icon: Trash2, danger: true, onSelect: () => emit('remove', picked.value.map(p => p.name)) }] : []),
    { label: 'New folder', icon: FolderPlus, separator: true, onSelect: () => emit('newFolder') },
    { label: 'Refresh', icon: RefreshCw, onSelect: () => sftp.refresh(props.index) },
  ]
})
</script>

<template>
  <section :aria-label="pane.label || 'Empty pane'" :data-testid="tid(index === 0 ? 'sftp-pane-left' : 'sftp-pane-right')" class="flex min-h-0 min-w-0 flex-col bg-bg" @keydown="onKey">
    <div class="flex h-10 shrink-0 items-center gap-2 border-b border-border px-2.5">
      <DropdownMenu>
        <DropdownMenuTrigger as-child>
          <Button variant="outline" size="sm" class="max-w-[180px] shrink-0 justify-start gap-2 bg-surface">
            <Loader v-if="pane.connecting" class="size-3.5 animate-spin" />
            <Laptop v-else-if="pane.endpoint === 'local'" class="size-3.5" />
            <Server v-else class="size-3.5" />
            <span class="truncate">{{ pane.label || 'Choose a place' }}</span>
            <ChevronDown class="size-3 text-muted" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" class="max-h-80 overflow-auto">
          <DropdownMenuItem @select="sftp.showLocal(index)"><Laptop />This computer</DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuLabel class="text-xs text-muted">Servers</DropdownMenuLabel>
          <DropdownMenuItem v-for="h in hosts.hosts" :key="h.id" @select="sftp.connect(index, h)"><Server />{{ h.label }}</DropdownMenuItem>
          <DropdownMenuItem v-if="!hosts.hosts.length" disabled>No servers yet</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <PathBar v-if="pane.endpoint" :path="pane.path" :remote="remote" @go="p => sftp.go(index, p)" />
      <span v-else class="flex-1" />
      <template v-if="pane.endpoint">
        <IconButton label="Copy to the other pane" size="sm" :disabled="!pane.selected.length || !other.endpoint" @click="copyOver"><ArrowLeftRight /></IconButton>
        <IconButton :label="pane.showHidden ? 'Hide hidden files' : 'Show hidden files'" :pressed="pane.showHidden" @click="sftp.toggleHidden(index)"><EyeOff v-if="pane.showHidden" /><Eye v-else /></IconButton>
        <IconButton label="New folder" @click="emit('newFolder')"><FolderPlus /></IconButton>
        <IconButton label="Refresh" @click="sftp.refresh(index)"><RefreshCw /></IconButton>
        <DropdownMenu>
          <DropdownMenuTrigger as-child><IconButton label="Actions" :tip="false"><Ellipsis /></IconButton></DropdownMenuTrigger>
          <DropdownMenuContent align="end"><ItemMenuList :items="fileMenu" mode="dropdown" /></DropdownMenuContent>
        </DropdownMenu>
      </template>
    </div>

    <div v-if="!pane.endpoint" class="grid flex-1 place-items-center p-6">
      <EmptyState :icon="Server" title="Choose a place" text="Pick this computer or a server to browse.">
        <Button variant="secondary" @click="sftp.showLocal(index)"><Laptop />This computer</Button>
      </EmptyState>
    </div>
    <div v-else-if="pane.error" class="grid flex-1 place-items-center p-6">
      <div class="flex max-w-sm flex-col items-center gap-2 text-center">
        <component :is="problem.icon" class="size-6 text-danger" aria-hidden="true" />
        <h3 v-if="problem.title" class="text-base">{{ problem.title }}</h3>
        <p role="alert" class="text-sm" :class="problem.title && 'text-muted'">{{ problem.text }}</p>
        <div class="flex gap-2">
          <Button v-if="pane.path" variant="secondary" size="sm" @click="sftp.go(index, parentPath(pane))">Go up</Button>
          <Button size="sm" @click="pane.hostId ? sftp.connect(index, hosts.hosts.find(h => h.id === pane.hostId)!) : sftp.refresh(index)">Try again</Button>
        </div>
      </div>
    </div>
    <ContextMenu v-else>
      <ContextMenuTrigger as-child>
        <div
          class="relative flex min-h-0 flex-1 flex-col"
          @dragover="sftp.dragging && sftp.dragging.pane !== index && ($event.preventDefault(), overDrop = true)"
          @dragleave="overDrop = false"
          @drop.prevent="overDrop = false; sftp.dropOn(index)"
        >
          <p v-if="pane.loading" class="absolute right-3 top-2 z-10 text-xs text-muted">Loading…</p>
          <FileTable
            :entries="rows"
            :selected="pane.selected"
            :sort="pane.sort"
            :show-mode="true"
            :drop-target="overDrop"
            @update:selected="n => (pane.selected = n)"
            @sort="setSort"
            @open="e => sftp.open(index, e)"
            @dragstart="names => (sftp.dragging = { pane: index, names })"
            @dragend="sftp.dragging = null"
            @context="e => (ctx = e)"
          />
          <p v-if="overDrop" class="m-3 rounded-md border border-dashed border-accent p-2.5 text-center text-xs text-accent">Drop to copy to {{ pane.path }}</p>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ItemMenuList :items="fileMenu" mode="context" />
      </ContextMenuContent>
    </ContextMenu>

    <div v-if="pane.endpoint" class="flex h-7 shrink-0 items-center border-t border-border px-2.5 text-xs text-muted">
      {{ foot }}<template v-if="remote"> · SFTP over {{ pane.label }}</template>
    </div>
  </section>
</template>
