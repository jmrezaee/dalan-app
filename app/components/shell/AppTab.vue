<script setup lang="ts">
// One tab in the TabBar (S01). The close button is a separate focusable button.
import { X } from '@lucide/vue'
import { cn } from '@/lib/utils'

const props = defineProps<{
  label: string
  active?: boolean
  dragging?: boolean
  dropTarget?: boolean
  unsaved?: boolean
  closable?: boolean
  /** Tabs that can be renamed (F2 or double-click) and dragged. */
  movable?: boolean
}>()
const emit = defineEmits<{ select: []; close: []; rename: [label: string]; dragStart: []; dragEnd: []; dropOn: [where: 'before' | 'after' | 'onto'] }>()

// ---- F2 / double-click rename
const editing = ref(false)
const draft = ref('')
const field = ref<HTMLInputElement | null>(null)
function startRename() {
  if (!props.movable) return
  draft.value = props.label
  editing.value = true
  nextTick(() => { field.value?.focus(); field.value?.select() })
}
function finishRename(save: boolean) {
  if (!editing.value) return
  editing.value = false
  if (save && draft.value.trim() && draft.value.trim() !== props.label) emit('rename', draft.value)
}

// ---- Drag: the middle half of a tab means "combine", the outer quarters mean "put before/after".
const zone = ref<'before' | 'after' | 'onto' | null>(null)
function zoneAt(e: DragEvent): 'before' | 'after' | 'onto' {
  const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
  const x = (e.clientX - r.left) / r.width
  return x < 0.25 ? 'before' : x > 0.75 ? 'after' : 'onto'
}
function onDragOver(e: DragEvent) {
  if (!props.movable || !e.dataTransfer?.types.includes('application/x-dalan-tab')) return
  e.preventDefault()
  zone.value = zoneAt(e)
}
function onDrop(e: DragEvent) {
  if (!props.movable) return
  e.preventDefault()
  const z = zoneAt(e)
  zone.value = null
  emit('dropOn', z)
}
</script>

<template>
  <div
    :class="cn(
      'group relative flex h-8 min-w-[120px] max-w-[220px] items-center gap-2 rounded-t-md px-2.5 text-sm',
      active ? 'bg-bg text-text' : 'text-muted hover:bg-surface-hover',
      dragging && 'opacity-90 shadow-drag',
      (dropTarget || zone === 'onto') && 'outline-2 outline-dashed outline-accent',
      zone === 'before' && 'shadow-[inset_2px_0_0_var(--accent)]',
      zone === 'after' && 'shadow-[inset_-2px_0_0_var(--accent)]',
    )"
    :draggable="movable && !editing"
    @dragstart="(e: DragEvent) => { e.dataTransfer?.setData('application/x-dalan-tab', label); emit('dragStart') }"
    @dragend="zone = null; emit('dragEnd')"
    @dragover="onDragOver"
    @dragleave="zone = null"
    @drop="onDrop"
    @auxclick.middle.prevent="closable && emit('close')"
  >
    <input
      v-if="editing"
      ref="field"
      v-model="draft"
      aria-label="Tab name"
      class="min-w-0 flex-1 rounded-sm border border-border-input bg-bg px-1 text-sm outline-none focus:border-accent"
      @keydown.enter.prevent="finishRename(true)"
      @keydown.esc.prevent="finishRename(false)"
      @blur="finishRename(true)"
    >
    <button v-else type="button" role="tab" :aria-selected="active" class="flex min-w-0 flex-1 items-center gap-2 text-left" @click="$emit('select')" @dblclick="startRename" @keydown.f2.prevent="startRename">
      <slot name="leading" />
      <span class="truncate">{{ label }}</span>
    </button>
    <span v-if="unsaved" class="size-1.5 shrink-0 rounded-full bg-muted" title="Unsaved changes" aria-label="Unsaved changes" />
    <button
      v-if="closable"
      type="button"
      :aria-label="`Close ${label}`"
      :class="cn('grid size-4 shrink-0 place-items-center rounded-sm hover:bg-surface-hover', active ? 'opacity-100' : 'opacity-0 group-hover:opacity-100 focus-visible:opacity-100')"
      @click="$emit('close')"
    >
      <X class="size-3" />
    </button>
    <span v-if="dropTarget || zone === 'onto'" class="absolute -bottom-5 left-0 text-2xs text-accent">Drop to split</span>
  </div>
</template>
