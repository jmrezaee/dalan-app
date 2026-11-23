<script setup lang="ts">
// S36 (read side): a table's Data view: the Where box, sorting by a column header, which columns show, the grid, the
// cell panel with the linked rows, and the pager with the planner's estimate ("Count exactly" asks for the real number).
// Nothing here changes data (editing is D4). The page itself is read inside a read-only transaction by Rust.
import { ChevronLeft, ChevronRight, Columns3, RefreshCw, Trash2 } from '@lucide/vue'
import { DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { PAGE_SIZE, type TableTab } from '~/stores/sql'

const props = defineProps<{ tabKey: string, tab: TableTab }>()
const store = useSqlStore(props.tabKey)
const t = computed(() => props.tab)
const VIEWS = [
  { id: 'data', label: 'Data', count: null },
  { id: 'structure', label: 'Structure', count: null },
  { id: 'indexes', label: 'Indexes', count: () => t.value.structure?.indexes.length ?? '' },
  { id: 'keys', label: 'Foreign keys', count: () => t.value.structure?.constraints.filter(c => c.kind === 'f').length ?? '' },
  { id: 'triggers', label: 'Triggers', count: () => t.value.structure?.triggers.length ?? '' },
  { id: 'ddl', label: 'DDL', count: null },
] as const

const sort = computed(() => t.value.sort[0] ?? null)
const allColumns = computed(() => t.value.info?.columns ?? [])
const shownNames = computed(() => (t.value.columns.length ? t.value.columns : allColumns.value.map(c => c.name)))
function toggleColumn(name: string, on: boolean) {
  const cur = new Set(shownNames.value)
  if (on) cur.add(name)
  else cur.delete(name)
  // Keep the table's own order; every column shown is the same as "all".
  const next = allColumns.value.map(c => c.name).filter(n => cur.has(n))
  if (!next.length) return
  void store.apply(t.value, { columns: next.length === allColumns.value.length ? [] : next })
}

const first = computed(() => t.value.page * PAGE_SIZE + 1)
const last = computed(() => t.value.page * PAGE_SIZE + t.value.rows.length)
const fmt = (n: number) => n.toLocaleString('en-US')
const total = computed(() => {
  if (t.value.exact !== null) return `${fmt(t.value.exact)}`
  // Without a filter the planner's number is a fair guess; with one it says nothing.
  if (!t.value.applied && t.value.estimate !== null && t.value.estimate >= last.value) return `${fmt(t.value.estimate)} (estimate)`
  return t.value.next ? 'more' : fmt(last.value)
})
// ---- Editing (F22): a table with a key, in a database that allows changes
const databases = useDatabasesStore()
const mode = computed(() => databases.rows.find(d => d.id === store.dbId)?.write_mode ?? 'confirm')
const editable = computed(() => !store.isMy && t.value.kind === 'table' && !!t.value.info?.columns.some(c => c.pk_position !== null) && mode.value !== 'read_only')
const changes = computed(() => store.changeCount(t.value))
const noKey = computed(() => !!t.value.info && !t.value.info.columns.some(c => c.pk_position !== null))

// ---- The cell panel and the linked rows
const cell = computed(() => {
  const s = t.value.selected
  if (!s) return null
  const c = t.value.rows[s.row]?.[s.col]
  const col = t.value.cols[s.col]
  return c && col ? { cell: c, col, row: s.row } : null
})
const cellText = computed(() => {
  const c = cell.value
  if (!c || c.cell.v === null) return null
  const raw = t.value.full?.text ?? c.cell.v
  if (c.cell.kind === 'json') {
    try { return JSON.stringify(JSON.parse(raw), null, 2) }
    catch { return raw }
  }
  return raw
})

/** A value as SQL text by its column's kind: numbers and booleans as they are, everything else quoted with its quotes doubled
 * (so a text key like 007 stays text). */
const lit = (v: string, kind: string) => (['int', 'decimal', 'float'].includes(kind) && /^-?\d+(\.\d+)?$/.test(v) ? v : kind === 'bool' && (v === 'true' || v === 'false') ? v : `'${v.replace(/'/g, '\'\'')}'`)
const q = (n: string) => `"${n.replace(/"/g, '""')}"`
/** The value and kind of a column in a row; nothing when the column isn't shown or its value was cut. */
function valueOf(name: string, row: number): { v: string, kind: string } | undefined {
  const i = t.value.cols.findIndex(c => c.name === name)
  const c = i < 0 ? undefined : t.value.rows[row]?.[i]
  return c && c.v !== null && !c.cut ? { v: c.v, kind: t.value.cols[i]!.kind } : undefined
}
interface Link { key: string, text: string, action: string, run: () => void }
const links = computed<Link[]>(() => {
  const c = cell.value
  const info = t.value.info
  if (!c || !info) return []
  const out: Link[] = []
  for (const fk of info.references) {
    const vals = fk.columns.map(n => valueOf(n, c.row))
    if (vals.some(v => !v)) continue
    const where = fk.columns.map((_, i) => `${q(fk.target_columns[i]!)} = ${lit(vals[i]!.v, vals[i]!.kind)}`).join(' and ')
    out.push({ key: `o-${fk.name}`, text: `${fk.table} ${vals.map(v => v!.v).join(', ')}`, action: 'Open row', run: () => void store.jump(fk.schema, fk.table, where) })
  }
  for (const fk of info.referenced_by) {
    const vals = fk.target_columns.map(n => valueOf(n, c.row))
    if (vals.some(v => !v)) continue
    const where = fk.columns.map((n, i) => `${q(n)} = ${lit(vals[i]!.v, vals[i]!.kind)}`).join(' and ')
    out.push({ key: `i-${fk.schema}.${fk.table}.${fk.name}`, text: `${fk.table} rows point here`, action: 'Show them', run: () => void store.jump(fk.schema, fk.table, where) })
  }
  return out
})
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col" :data-testid="tid(`sql-table-${t.table}`)">
    <div v-if="!store.isMy" role="group" aria-label="View" class="flex h-9 shrink-0 items-center gap-0.5 border-b border-border px-2">
      <button
        v-for="v in (store.isMy ? VIEWS.slice(0, 1) : VIEWS)" :key="v.id" type="button" :aria-current="t.view === v.id ? 'true' : undefined"
        :class="['h-7 cursor-default rounded-md px-2.5 text-sm text-muted outline-none hover:text-text focus-visible:ring-2 focus-visible:ring-accent', t.view === v.id && 'bg-surface-hover text-text']" @click="store.setView(t, v.id)"
      >{{ v.label }}<span v-if="v.count" class="ml-1 text-2xs text-muted">{{ v.count() }}</span></button>
    </div>
    <SqlStructureView v-if="t.view !== 'data'" :tab-key="tabKey" :tab="t" />
    <div v-show="t.view === 'data'" class="flex min-h-0 flex-1 flex-col">
    <form class="flex h-11 shrink-0 items-center gap-2 border-b border-border px-3" @submit.prevent="store.apply(t)">
      <label :for="`sql-where-${tabKey}-${t.id}`" class="text-xs text-muted">Where</label>
      <Input :id="`sql-where-${tabKey}-${t.id}`" v-model="t.filter" class="h-7 flex-1 font-mono text-xs" placeholder="status = 'open' and total > 100" autocomplete="off" spellcheck="false" :aria-invalid="!!t.error && !!t.filter" />
      <span v-if="sort" class="inline-flex h-6 items-center rounded-full border border-border px-2 text-2xs text-muted" :title="'Sorted by ' + sort.column">{{ sort.column }} {{ sort.descending ? '↓' : '↑' }}</span>
      <DropdownMenu>
        <DropdownMenuTrigger as-child><Button type="button" variant="secondary" size="sm"><Columns3 />Columns {{ shownNames.length }}/{{ allColumns.length }}</Button></DropdownMenuTrigger>
        <DropdownMenuContent align="end" class="max-h-80 overflow-auto">
          <DropdownMenuLabel>Columns</DropdownMenuLabel>
          <DropdownMenuSeparator />
          <DropdownMenuCheckboxItem v-for="c in allColumns" :key="c.name" :model-value="shownNames.includes(c.name)" @update:model-value="(v: boolean) => toggleColumn(c.name, v)" @select.prevent>{{ c.name }}</DropdownMenuCheckboxItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <Button type="submit" size="sm" :loading="t.loading">Apply</Button>
      <IconButton label="Refresh" type="button" :disabled="changes > 0" @click="store.refresh(t)"><RefreshCw /></IconButton>
    </form>
    <p v-if="t.error" role="alert" class="shrink-0 border-b border-border px-3 py-1.5 text-xs text-danger">{{ t.error }}</p>
    <p v-else-if="noKey" role="status" class="shrink-0 border-b border-border px-3 py-1.5 text-xs text-muted">This table has no key, so rows can't be matched safely. Use a query instead when editing arrives.</p>

    <div class="flex min-h-0 flex-1">
      <div class="flex min-w-0 flex-1 flex-col">
        <DataGrid
          v-if="t.cols.length" :label="`${t.schema}.${t.table} rows`" :columns="t.cols" :rows="t.rows" :selected="t.selected"
          :sort-column="sort?.column ?? null" :sort-descending="sort?.descending ?? false" :editable="editable" :edited="t.edits" :deleted-rows="t.deleted"
          @edit="(r: number, c: number, v: string | null) => store.setCell(t, r, c, v)" @select="(r: number, c: number) => store.select(t, r, c)" @sort="(c: string) => store.sortBy(t, c)"
        />
        <div v-else-if="t.loading" class="p-4 text-sm text-muted" aria-busy="true">Reading {{ t.table }}…</div>
        <div v-if="t.cols.length && !t.rows.length && !t.loading" class="p-4 text-sm text-muted">{{ t.applied ? 'No rows match this filter. Clear it?' : 'This table has no rows yet.' }}</div>
      </div>

      <aside v-if="cell" class="flex w-72 shrink-0 flex-col gap-3 overflow-auto border-l border-border p-3 text-sm" aria-label="Cell">
        <div>
          <h3 class="truncate font-mono text-xs font-medium">{{ cell.col.name }}</h3>
          <p class="text-xs text-muted">{{ cell.col.type_name }} · row {{ t.page * PAGE_SIZE + cell.row + 1 }}</p>
        </div>
        <pre v-if="cellText !== null" class="max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-md border border-border bg-surface p-2 font-mono text-xs" :data-testid="tid('cell-value')">{{ cellText }}</pre>
        <p v-else class="text-xs italic text-muted">NULL</p>
        <p v-if="cell.cell.cut && t.full?.loading" class="text-xs text-muted" role="status">Loading the whole value…</p>
        <p v-else-if="cell.cell.cut && !t.full" class="text-xs text-muted">Cut at 64 KB. This table has no key to load the rest with.</p>
        <div v-if="links.length" class="flex flex-col gap-1.5">
          <h4 class="text-2xs font-medium uppercase tracking-wider text-muted">Linked</h4>
          <div v-for="l in links" :key="l.key" class="flex items-center gap-2 rounded-md border border-border px-2 py-1.5">
            <span class="min-w-0 flex-1 truncate text-xs">{{ l.text }}</span>
            <Button variant="secondary" size="xs" @click="l.run()">{{ l.action }}</Button>
          </div>
        </div>
      </aside>
    </div>

    <div v-if="changes" class="flex h-10 shrink-0 items-center gap-3 border-t border-warning/40 bg-warning/10 px-3 text-sm" role="status" :data-testid="tid('sql-changes')">
      <span class="flex-1"><b class="font-medium">{{ changes }} {{ changes === 1 ? 'change' : 'changes' }}</b> not committed. Nothing is written until you review and commit.</span>
      <Button variant="ghost" size="sm" @click="store.discardEdits(t)">Discard</Button>
      <Button size="sm" @click="store.review(t)">Review changes</Button>
    </div>
    <p v-else-if="editable && t.rows.length" class="shrink-0 border-t border-border px-3 py-1 text-2xs text-muted">Select a cell and press Enter to edit it. Delete makes it NULL.</p>
    <SqlCommitDialog :tab-key="tabKey" :tab="t" />
    <footer class="flex h-9 shrink-0 items-center gap-3 border-t border-border px-3 text-xs text-muted">
      <span v-if="t.rows.length" :data-testid="tid('sql-range')">Rows {{ fmt(first) }}–{{ fmt(last) }} of {{ total }}<template v-if="t.elapsedMs !== null"> · {{ t.elapsedMs }} ms</template></span>
      <span v-else-if="t.cols.length">0 rows</span>
      <Button v-if="t.exact === null && t.rows.length" variant="ghost" size="xs" @click="store.countExactly(t)">Count exactly</Button>
      <Button v-if="editable && t.selected && cell" variant="ghost" size="xs" @click="store.toggleDelete(t, t.selected.row)"><Trash2 />{{ t.deleted.includes(t.selected.row) ? 'Keep row' : 'Delete row' }}</Button>
      <span class="ml-auto flex items-center gap-1">
        <IconButton label="Previous page" :disabled="t.page === 0 || t.loading || changes > 0" @click="store.prevPage(t)"><ChevronLeft /></IconButton>
        <span class="min-w-12 text-center">Page {{ t.page + 1 }}</span>
        <IconButton label="Next page" :disabled="!t.next || t.loading || changes > 0" @click="store.nextPage(t)"><ChevronRight /></IconButton>
      </span>
    </footer>
    </div>
  </div>
</template>
