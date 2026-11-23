<script setup lang="ts">
// The shared data grid (spec section 4): SQL tables, SQL results and, later, MongoDB's table view. Virtualised rows,
// a sticky header, resizable columns, and the keyboard (arrows, Home/End, PageUp/PageDown, ⌘C copies the cell,
// ⇧⌘C the row as tab-separated text). It only shows and selects: editing arrives with D4.
import { ArrowDown, ArrowUp, KeyRound } from '@lucide/vue'
import { cn } from '@/lib/utils'
import type { SqlCell_Serialize as Cell, SqlColumn_Serialize as SqlColumn } from '~/types/bindings'

const props = defineProps<{
  label: string
  columns: SqlColumn[]
  rows: Cell[][]
  selected: { row: number, col: number } | null
  /** The column the data is sorted by, if any. */
  sortColumn?: string | null
  sortDescending?: boolean
  /** Cells can be edited (F22): Enter, F2 or a double-click opens the cell; Delete makes it NULL. */
  editable?: boolean
  /** Pending edits by `row:col`: the new text (`null` is NULL). The cell shows it as changed. */
  edited?: Record<string, string | null>
  /** Rows marked for deletion, struck through. */
  deletedRows?: number[]
}>()
const emit = defineEmits<{ select: [row: number, col: number], sort: [column: string], edit: [row: number, col: number, value: string | null] }>()

// ---- Editing one cell
const editing = ref<{ row: number, col: number, value: string } | null>(null)
const cellOf = (row: number, col: number) => props.rows[row]?.[col]
const editedKey = (row: number, col: number) => `${row}:${col}`
const isEdited = (row: number, col: number) => !!props.edited && editedKey(row, col) in props.edited
function shownValue(row: number, col: number, c: Cell): Cell {
  return isEdited(row, col) ? { kind: props.edited![editedKey(row, col)] === null ? 'null' : c.kind, v: props.edited![editedKey(row, col)]!, cut: false } : c
}
function canEditCell(row: number, col: number) {
  const c = cellOf(row, col)
  return !!props.editable && !!c && !c.cut && !props.deletedRows?.includes(row)
}
function startEdit(row: number, col: number) {
  if (!canEditCell(row, col)) return
  const c = shownValue(row, col, cellOf(row, col)!)
  editing.value = { row, col, value: c.v ?? '' }
  emit('select', row, col)
  nextTick(() => scroller.value?.querySelector<HTMLInputElement>('input[data-cell-editor]')?.focus())
}
function finishEdit(save: boolean) {
  const e = editing.value
  editing.value = null
  if (save && e) emit('edit', e.row, e.col, e.value)
  nextTick(() => scroller.value?.focus())
}

const ROW = 28
const { scroller, onScroll, from, to, padTop, padBottom, reveal } = useVirtualRows(computed(() => props.rows.length), ROW)
const shown = computed(() => props.rows.slice(from.value, to.value))

const NUMERIC = new Set(['int', 'decimal', 'float'])
const defaultWidth = (c: SqlColumn) => (NUMERIC.has(c.kind) ? 110 : c.kind === 'json' ? 260 : c.kind === 'uuid' ? 290 : c.kind === 'datetime' ? 190 : c.kind === 'bool' ? 80 : 180)
const widths = ref<number[]>([])
watch(() => props.columns.map(c => c.name).join('\u0000'), () => { widths.value = props.columns.map(defaultWidth) }, { immediate: true })
const template = computed(() => `48px ${widths.value.map(w => `${w}px`).join(' ')}`)
const totalWidth = computed(() => 48 + widths.value.reduce((a, b) => a + b, 0))

// Dragging the edge of a header resizes its column.
function startResize(e: PointerEvent, col: number) {
  const startX = e.clientX
  const start = widths.value[col] ?? 120
  const move = (ev: PointerEvent) => { widths.value[col] = Math.max(60, start + ev.clientX - startX) }
  const up = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up) }
  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', up)
}

const text = (c: Cell) => (c.v === null ? 'NULL' : c.cut ? `${c.v.slice(0, 200)}…` : c.v)
function shownText(c: Cell) {
  if (c.v === null) return 'NULL'
  const flat = c.v.length > 400 ? c.v.slice(0, 400) : c.v
  return c.cut ? `${flat}…` : flat
}
const alignRight = (c: Cell) => NUMERIC.has(c.kind)
const isSelected = (row: number, col: number) => props.selected?.row === from.value + row && props.selected.col === col

function onKey(e: KeyboardEvent) {
  const s = props.selected ?? { row: 0, col: 0 }
  const rows = props.rows.length
  const cols = props.columns.length
  if (!rows || !cols) return
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'c') {
    e.preventDefault()
    const row = props.rows[s.row]
    if (!row) return
    const out = e.shiftKey ? row.map(c => c.v ?? '').join('\t') : (row[s.col]?.v ?? '')
    void navigator.clipboard.writeText(out)
    return
  }
  if (props.editable && props.selected) {
    if (e.key === 'Enter' || e.key === 'F2') { e.preventDefault(); startEdit(props.selected.row, props.selected.col); return }
    if (e.key === 'Delete' && canEditCell(props.selected.row, props.selected.col)) { e.preventDefault(); emit('edit', props.selected.row, props.selected.col, null); return }
  }
  // The first arrow key selects the first cell instead of moving from it.
  if (!props.selected && e.key.startsWith('Arrow')) {
    e.preventDefault()
    emit('select', 0, 0)
    return
  }
  let { row, col } = s
  switch (e.key) {
    case 'ArrowDown': row++; break
    case 'ArrowUp': row--; break
    case 'ArrowRight': col++; break
    case 'ArrowLeft': col--; break
    case 'PageDown': row += 10; break
    case 'PageUp': row -= 10; break
    case 'Home': if (e.ctrlKey || e.metaKey) row = 0; else col = 0; break
    case 'End': if (e.ctrlKey || e.metaKey) row = rows - 1; else col = cols - 1; break
    default: return
  }
  e.preventDefault()
  row = Math.min(rows - 1, Math.max(0, row))
  col = Math.min(cols - 1, Math.max(0, col))
  emit('select', row, col)
  reveal(row)
}
</script>

<template>
  <div
    ref="scroller" role="grid" tabindex="0" :aria-label="label" :aria-rowcount="rows.length + 1" :aria-colcount="columns.length + 1"
    class="relative min-h-0 flex-1 overflow-auto text-xs outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" :data-testid="tid('data-grid')"
    @scroll.passive="onScroll" @keydown="onKey"
  >
    <div :style="{ width: `${totalWidth}px` }">
      <div role="row" class="sticky top-0 z-10 grid h-7 border-b border-border bg-bg text-muted" :style="{ gridTemplateColumns: template }">
        <div role="columnheader" class="border-r border-border px-2 leading-7" aria-label="Row" />
        <div v-for="(c, i) in columns" :key="c.name" role="columnheader" :aria-sort="sortColumn === c.name ? (sortDescending ? 'descending' : 'ascending') : 'none'" class="relative flex min-w-0 items-center border-r border-border">
          <button type="button" class="flex h-full min-w-0 flex-1 items-center gap-1 px-2 text-left font-medium hover:text-text" :title="`${c.name} · ${c.type_name}${c.nullable ? '' : ' · not null'}`" @click="emit('sort', c.name)">
            <KeyRound v-if="c.pk_position !== null" class="size-3 shrink-0 text-accent" aria-label="Primary key" />
            <span class="truncate">{{ c.name }}</span>
            <span class="truncate font-normal text-muted">{{ c.type_name }}</span>
            <ArrowUp v-if="sortColumn === c.name && !sortDescending" class="size-3 shrink-0 text-accent" aria-hidden="true" />
            <ArrowDown v-else-if="sortColumn === c.name" class="size-3 shrink-0 text-accent" aria-hidden="true" />
          </button>
          <span class="absolute inset-y-0 right-0 z-10 w-1.5 cursor-col-resize hover:bg-accent/40" aria-hidden="true" @pointerdown.prevent="startResize($event, i)" />
        </div>
      </div>
      <div :style="{ height: `${padTop}px` }" />
      <div v-for="(r, ri) in shown" :key="from + ri" role="row" :aria-rowindex="from + ri + 2" :aria-label="deletedRows?.includes(from + ri) ? 'Marked for deletion' : undefined" :class="cn('grid border-b border-surface-raised hover:bg-surface', deletedRows?.includes(from + ri) && 'bg-danger/10 line-through opacity-70')" :style="{ gridTemplateColumns: template, height: `${ROW}px` }">
        <div role="rowheader" class="border-r border-surface-raised px-2 text-right font-mono leading-7 text-muted">{{ from + ri + 1 }}</div>
        <div
          v-for="(c0, ci) in r" :key="ci" role="gridcell" :aria-selected="isSelected(ri, ci)" :title="c0.v !== null && c0.v.length > 60 ? text(c0) : undefined"
          :class="cn('min-w-0 truncate border-r border-surface-raised px-2 font-mono leading-7', alignRight(shownValue(from + ri, ci, c0)) && 'text-right', shownValue(from + ri, ci, c0).v === null && 'italic text-muted', c0.kind === 'bool' && c0.v !== null && 'text-ansi-blue', isEdited(from + ri, ci) && 'bg-warning/15', isSelected(ri, ci) && 'bg-accent/15 ring-1 ring-inset ring-accent')"
          @click="emit('select', from + ri, ci)" @dblclick="startEdit(from + ri, ci)"
        >
          <input
            v-if="editing && editing.row === from + ri && editing.col === ci" v-model="editing.value" data-cell-editor class="size-full bg-transparent font-mono outline-none" :aria-label="`Edit ${columns[ci]?.name}`"
            @keydown.stop @keydown.enter.prevent="finishEdit(true)" @keydown.esc.prevent="finishEdit(false)" @blur="finishEdit(true)"
          >
          <template v-else>{{ shownText(shownValue(from + ri, ci, c0)) }}</template>
        </div>
      </div>
      <div :style="{ height: `${padBottom}px` }" />
    </div>
  </div>
</template>
