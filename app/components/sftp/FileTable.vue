<script setup lang="ts">
// S21: the rows of one pane. Click selects (⌘/Ctrl adds, Shift extends), double-click opens a folder,
// Enter opens, and selected rows can be dragged to the other pane.
import { File, Folder, Link2 } from '@lucide/vue'
import type { FsEntry } from '~/types/bindings'
import { cn } from '@/lib/utils'
import { formatBytes, formatMode, formatModified } from '~/utils/format'
import type { Pane } from '~/stores/sftp'

const props = defineProps<{ entries: FsEntry[], selected: string[], sort: Pane['sort'], showMode: boolean, dropTarget?: boolean }>()
const emit = defineEmits<{
  'update:selected': [names: string[]]
  'sort': [by: Pane['sort']['by']]
  'open': [entry: FsEntry]
  'dragstart': [names: string[]]
  'dragend': []
  'context': [entry: FsEntry | null, event: MouseEvent]
}>()

let anchor = ''

// ---- Long folders: past this many rows only the ones in view are drawn (rows are a fixed 28px).
const VIRTUAL_OVER = 500
const ROW_H = 28
const scroller = ref<HTMLElement | null>(null)
const scrollTop = ref(0)
const viewH = ref(600)
const virtual = computed(() => props.entries.length > VIRTUAL_OVER)
const range = computed(() => {
  if (!virtual.value) return { from: 0, to: props.entries.length }
  const from = Math.max(0, Math.floor(scrollTop.value / ROW_H) - 15)
  return { from, to: Math.min(props.entries.length, from + Math.ceil(viewH.value / ROW_H) + 30) }
})
const shown = computed(() => (virtual.value ? props.entries.slice(range.value.from, range.value.to) : props.entries))
function onScroll() {
  if (!scroller.value) return
  scrollTop.value = scroller.value.scrollTop
  viewH.value = scroller.value.clientHeight
}
onMounted(onScroll)
watch(() => props.entries, () => { if (scroller.value && scroller.value.scrollTop > props.entries.length * ROW_H) scroller.value.scrollTop = 0; onScroll() })
function select(e: FsEntry, ev: MouseEvent | KeyboardEvent) {
  if (ev.shiftKey && anchor) {
    const names = props.entries.map(x => x.name)
    const [a, b] = [names.indexOf(anchor), names.indexOf(e.name)].sort((x, y) => x - y)
    emit('update:selected', names.slice(a!, b! + 1))
    return
  }
  anchor = e.name
  if (ev.metaKey || ev.ctrlKey) {
    emit('update:selected', props.selected.includes(e.name) ? props.selected.filter(n => n !== e.name) : [...props.selected, e.name])
  }
  else emit('update:selected', [e.name])
}

// The row's handler runs first and lets the event bubble up to the pane's ContextMenu (which opens the menu and
// cancels the browser's own). Here we only handle clicks on empty space, so the row's choice isn't overwritten.
function onBlankContext(ev: MouseEvent) {
  if (!(ev.target as HTMLElement).closest('tr[role=row]')) emit('context', null, ev)
}

function start(ev: DragEvent, e: FsEntry) {
  const names = props.selected.includes(e.name) ? props.selected : [e.name]
  emit('update:selected', names)
  ev.dataTransfer?.setData('text/plain', names.join('\n'))
  if (ev.dataTransfer) ev.dataTransfer.effectAllowed = 'copy'
  emit('dragstart', names)
}

const arrow = (by: Pane['sort']['by']) => (props.sort.by === by ? (props.sort.desc ? ' ↓' : ' ↑') : '')
</script>

<template>
  <div ref="scroller" :class="cn('flex-1 overflow-auto', dropTarget && 'ring-2 ring-inset ring-accent')" @contextmenu="onBlankContext" @scroll.passive="onScroll">
    <table class="w-full border-collapse text-sm" role="grid">
      <thead>
        <tr class="sticky top-0 bg-bg text-left text-xs font-medium text-muted">
          <th class="h-7 border-b border-border px-2.5" :aria-sort="sort.by === 'name' ? (sort.desc ? 'descending' : 'ascending') : undefined"><button type="button" @click="emit('sort', 'name')">Name{{ arrow('name') }}</button></th>
          <th class="h-7 border-b border-border px-2.5 text-right"><button type="button" @click="emit('sort', 'size')">Size{{ arrow('size') }}</button></th>
          <th class="h-7 border-b border-border px-2.5"><button type="button" @click="emit('sort', 'mtime')">Modified{{ arrow('mtime') }}</button></th>
          <th v-if="showMode" class="h-7 border-b border-border px-2.5">Permissions</th>
        </tr>
      </thead>
      <tbody>
        <tr v-if="virtual && range.from > 0" aria-hidden="true" :style="{ height: `${range.from * ROW_H}px` }"><td :colspan="showMode ? 4 : 3" /></tr>
        <tr
          v-for="e in shown"
          :key="e.name"
          :data-testid="tid(`file-row-${e.name}`)"
          role="row"
          tabindex="0"
          draggable="true"
          :aria-selected="selected.includes(e.name)"
          :class="cn('h-7 cursor-default select-none whitespace-nowrap outline-none [-webkit-user-drag:element]', selected.includes(e.name) ? 'bg-accent/15' : 'hover:bg-surface-hover')"
          @click="select(e, $event)"
          @dblclick="emit('open', e)"
          @keydown.enter="emit('open', e)"
          @keydown.space.prevent="select(e, $event)"
          @dragstart="start($event, e)"
          @dragend="emit('dragend')"
          @contextmenu="(select(e, $event), emit('context', e, $event))"
        >
          <td class="px-2.5">
            <span class="inline-flex items-center gap-2">
              <Folder v-if="e.kind === 'dir'" class="size-4 text-accent" aria-hidden="true" />
              <Link2 v-else-if="e.symlink" class="size-4 text-muted" aria-hidden="true" />
              <File v-else class="size-4 text-muted" aria-hidden="true" />
              {{ e.name }}
            </span>
          </td>
          <td class="px-2.5 text-right tabular-nums text-muted">{{ e.kind === 'file' ? formatBytes(e.size ?? 0) : '—' }}</td>
          <td class="px-2.5 text-muted">{{ formatModified(e.mtime) }}</td>
          <td v-if="showMode" class="px-2.5 font-mono text-xs text-muted">{{ formatMode(e.mode, e.kind, e.symlink) }}</td>
        </tr>
        <tr v-if="virtual && range.to < entries.length" aria-hidden="true" :style="{ height: `${(entries.length - range.to) * ROW_H}px` }"><td :colspan="showMode ? 4 : 3" /></tr>
      </tbody>
    </table>
    <p v-if="!entries.length" class="px-4 py-6 text-center text-sm text-muted">This folder is empty.</p>
  </div>
</template>
