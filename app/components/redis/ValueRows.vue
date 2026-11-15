<script setup lang="ts">
// The table every collection viewer draws (hash, list, set, sorted set, stream): mono cells, virtualised.
// A cell is a value's bytes (text as it is, binary as hex with a tag) or a plain label (a position, a score, an id).
// With `edit` or `removable` it is also the editor (R3): one item at a time, ending in "Review change" or "Save".
// It only says what the person wants (`save`, `remove`); the wrappers turn that into a write for Rust's gate.
import { Pencil, Trash2 } from '@lucide/vue'
import { Input } from '@/components/ui/input'
import type { RedisBytes } from '~/types/bindings'
import { bytesOf, bytesText } from '~/utils/redisBytes'
import { cn } from '@/lib/utils'

export type Cell = RedisBytes | string
const props = defineProps<{
  label: string
  columns: { label: string, width?: string }[]
  rows: Cell[][]
  /** The columns that become inputs when a row is edited. Leave out for "no editing". */
  edit?: number[]
  removable?: boolean
  /** "Review change" (Confirm mode) or "Save" (Open mode). */
  saveLabel?: string
  /** What one row is called, for the buttons' names: "field", "member". */
  item?: string
}>()
const emit = defineEmits<{ save: [index: number, values: string[]], remove: [index: number] }>()

const ROW = 30
const { scroller, onScroll, from, to, padTop, padBottom, reveal } = useVirtualRows(computed(() => props.rows.length), ROW)
const shown = computed(() => props.rows.slice(from.value, to.value))
const focused = ref(0)
watch(() => props.rows.length, (n) => { if (focused.value >= n) focused.value = Math.max(0, n - 1) })

const isBinary = (c: Cell) => typeof c !== 'string' && c.kind === 'binary'
const show = (c: Cell) => (typeof c === 'string' ? c : c.kind === 'text' ? c.text : Array.from(bytesOf(c).subarray(0, 64), n => n.toString(16).padStart(2, '0')).join(' '))
const title = (c: Cell) => (typeof c === 'string' ? c : bytesText(c).slice(0, 2000))
const actions = computed(() => !!props.edit || props.removable)
const grid = computed(() => ({ gridTemplateColumns: [...props.columns.map(c => c.width ?? 'minmax(0, 1fr)'), ...(actions.value ? ['64px'] : [])].join(' ') }))
const rowName = (r: Cell[]) => title(r[0] ?? '').slice(0, 80)

// ---- Editing: `index` is the row, or -1 for the "add" row at the top. `fixed[j]` is text shown instead of an input.
const editing = ref<{ index: number, values: string[], fixed: (string | null)[], hints: string[] } | null>(null)
const canEdit = (r: Cell[]) => !!props.edit?.length && props.edit.every(j => !isBinary(r[j] ?? ''))
function startEdit(i: number) {
  const r = props.rows[i]
  if (!r || !canEdit(r)) return
  focused.value = i
  editing.value = { index: i, values: r.map(show), fixed: r.map((c, j) => (props.edit!.includes(j) ? null : show(c))), hints: [] }
  focusInput()
}
/** The wrappers' "Add" buttons: one input per `null`, fixed text otherwise ("start", "end"). */
function startAdd(fixed: (string | null)[], hints: string[] = []) {
  editing.value = { index: -1, values: fixed.map(() => ''), fixed, hints }
  focusInput()
}
const cancel = () => { editing.value = null }
function save() {
  const e = editing.value
  if (e) emit('save', e.index, e.values.map((v, j) => e.fixed[j] ?? v))
}
const editRow = ref<HTMLElement | null>(null)
const focusInput = () => nextTick(() => (Array.isArray(editRow.value) ? editRow.value[0] : editRow.value)?.querySelector('input')?.focus())
// The rows changed (the write went through and the key was read again, or the filter moved): the edit is over.
watch(() => props.rows, cancel)
defineExpose({ startAdd })

function onKey(e: KeyboardEvent) {
  if (editing.value) return
  if (e.key === 'Enter' && props.edit) {
    e.preventDefault()
    return startEdit(focused.value)
  }
  if ((e.key === 'Delete' || e.key === 'Backspace') && props.removable && props.rows.length) {
    e.preventDefault()
    return emit('remove', focused.value)
  }
  const d = e.key === 'ArrowDown' ? 1 : e.key === 'ArrowUp' ? -1 : e.key === 'PageDown' ? 20 : e.key === 'PageUp' ? -20 : 0
  if (!d) return
  e.preventDefault()
  focused.value = Math.min(props.rows.length - 1, Math.max(0, focused.value + d))
  reveal(focused.value)
}
</script>

<template>
  <div ref="scroller" role="table" :aria-label="label" :aria-rowcount="rows.length + 1" tabindex="0" class="min-h-0 flex-1 overflow-auto outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" @scroll="onScroll" @keydown="onKey">
    <div role="row" class="sticky top-0 z-10 grid h-7 items-center border-b border-border bg-bg text-xs font-medium text-muted" :style="grid">
      <span v-for="c in columns" :key="c.label" role="columnheader" class="px-2.5">{{ c.label }}</span>
      <span v-if="actions" role="columnheader"><span class="sr-only">Actions</span></span>
    </div>

    <!-- One template for the row being added (index -1, on top) and the row being edited (in place). -->
    <form v-if="editing?.index === -1" ref="editRow" role="row" class="sticky top-7 z-10 flex h-9 items-center gap-2 border-b border-border bg-surface px-2.5" :data-testid="tid('redis-edit-row')" @submit.prevent="save" @keydown.esc.stop.prevent="cancel" @keydown.stop>
      <template v-for="(f, j) in editing.fixed" :key="j">
        <span v-if="f !== null" role="cell" class="max-w-[30%] shrink-0 truncate font-mono text-xs text-muted">{{ f }}</span>
        <span v-else role="cell" class="min-w-0 flex-1"><Input v-model="editing.values[j]" :aria-label="columns[j]?.label.replace(' ↓', '')" :placeholder="editing.hints[j] ?? columns[j]?.label.replace(' ↓', '')" class="h-7 font-mono text-xs" autocomplete="off" spellcheck="false" /></span>
      </template>
      <span role="cell" class="flex shrink-0 gap-1.5"><Button variant="secondary" size="sm" type="button" @click="cancel">Cancel</Button><Button size="sm" type="submit">{{ saveLabel ?? 'Save' }}</Button></span>
    </form>

    <div :style="{ height: `${padTop}px` }" aria-hidden="true" />
    <template v-for="(r, i) in shown" :key="from + i">
      <form v-if="editing?.index === from + i" ref="editRow" role="row" :aria-rowindex="from + i + 2" class="flex h-[30px] items-center gap-2 border-b border-surface-raised bg-surface px-2.5" :data-testid="tid('redis-edit-row')" @submit.prevent="save" @keydown.esc.stop.prevent="cancel" @keydown.stop>
        <template v-for="(f, j) in editing.fixed" :key="j">
          <span v-if="f !== null" role="cell" class="max-w-[30%] shrink-0 truncate font-mono text-xs text-muted" :title="f">{{ f }}</span>
          <span v-else role="cell" class="min-w-0 flex-1"><Input v-model="editing.values[j]" :aria-label="columns[j]?.label.replace(' ↓', '')" class="h-6 font-mono text-xs" autocomplete="off" spellcheck="false" /></span>
        </template>
        <span role="cell" class="flex shrink-0 gap-1.5"><Button variant="secondary" size="sm" type="button" class="h-6" @click="cancel">Cancel</Button><Button size="sm" type="submit" class="h-6">{{ saveLabel ?? 'Save' }}</Button></span>
      </form>
      <div
        v-else
        role="row"
        :aria-rowindex="from + i + 2"
        :class="cn('group grid h-[30px] items-center border-b border-surface-raised', from + i === focused && 'bg-surface')"
        :style="grid"
        @click="focused = from + i"
        @dblclick="startEdit(from + i)"
      >
        <span v-for="(c, j) in r" :key="j" role="cell" :class="cn('truncate px-2.5 font-mono text-xs', (typeof c === 'string' || isBinary(c)) && 'text-muted')" :title="title(c)">
          {{ show(c) }}<span v-if="isBinary(c)" class="ml-2 rounded border border-border px-1 font-sans text-2xs">binary</span>
        </span>
        <span v-if="actions" role="cell" :class="cn('flex justify-end pr-1.5 opacity-0 focus-within:opacity-100 group-hover:opacity-100', from + i === focused && 'opacity-100')">
          <IconButton v-if="canEdit(r)" size="xs" :tip="false" :label="`Edit ${item ?? 'item'} ${rowName(r)}`" @click.stop="startEdit(from + i)"><Pencil /></IconButton>
          <IconButton v-if="removable" size="xs" :tip="false" :label="`Remove ${item ?? 'item'} ${rowName(r)}`" @click.stop="emit('remove', from + i)"><Trash2 /></IconButton>
        </span>
      </div>
    </template>
    <div :style="{ height: `${padBottom}px` }" aria-hidden="true" />
  </div>
</template>
