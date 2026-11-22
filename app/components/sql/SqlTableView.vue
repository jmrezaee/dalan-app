<script setup lang="ts">
// S36 (read side): a table's Data view: the Where box, sorting by a column header, which columns show, the grid, the
// cell panel with the linked rows, and the pager with the planner's estimate ("Count exactly" asks for the real number).
// Nothing here changes data (editing is D4). The page itself is read inside a read-only transaction by Rust.
import { ChevronLeft, ChevronRight, Columns3, RefreshCw } from '@lucide/vue'
import { DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { PAGE_SIZE, type TableTab } from '~/stores/sql'

const props = defineProps<{ tabKey: string, tab: TableTab }>()
const store = useSqlStore(props.tabKey)
const t = computed(() => props.tab)

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
  t.value.columns = next.length === allColumns.value.length ? [] : next
  void store.apply(t.value)
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

/** `col = value` for a jump: numbers and booleans as they are, everything else quoted with its quotes doubled. */
const lit = (v: string) => (/^-?\d+(\.\d+)?$/.test(v) || v === 'true' || v === 'false' ? v : `'${v.replace(/'/g, '\'\'')}'`)
const q = (n: string) => `"${n.replace(/"/g, '""')}"`
function valueOf(name: string, row: number) {
  const i = t.value.cols.findIndex(c => c.name === name)
  return i < 0 ? undefined : t.value.rows[row]?.[i]?.v
}
interface Link { key: string, text: string, action: string, run: () => void }
const links = computed<Link[]>(() => {
  const c = cell.value
  const info = t.value.info
  if (!c || !info) return []
  const out: Link[] = []
  for (const fk of info.references) {
    const vals = fk.columns.map(n => valueOf(n, c.row))
    if (vals.some(v => v == null)) continue
    const where = fk.columns.map((_, i) => `${q(fk.target_columns[i]!)} = ${lit(vals[i]!)}`).join(' and ')
    out.push({ key: `o-${fk.name}`, text: `${fk.table} ${vals.join(', ')}`, action: 'Open row', run: () => void store.jump(fk.schema, fk.table, where) })
  }
  for (const fk of info.referenced_by) {
    const vals = fk.target_columns.map(n => valueOf(n, c.row))
    if (vals.some(v => v == null)) continue
    const where = fk.columns.map((n, i) => `${q(n)} = ${lit(vals[i]!)}`).join(' and ')
    out.push({ key: `i-${fk.schema}.${fk.table}.${fk.name}`, text: `${fk.table} rows point here`, action: 'Show them', run: () => void store.jump(fk.schema, fk.table, where) })
  }
  return out
})
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col" :data-testid="tid(`sql-table-${t.table}`)">
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
      <IconButton label="Refresh" type="button" @click="store.refresh(t)"><RefreshCw /></IconButton>
    </form>
    <p v-if="t.error" role="alert" class="shrink-0 border-b border-border px-3 py-1.5 text-xs text-danger">{{ t.error }}</p>
    <p v-else-if="noKey" role="status" class="shrink-0 border-b border-border px-3 py-1.5 text-xs text-muted">This table has no key, so rows can't be matched safely. Use a query instead when editing arrives.</p>

    <div class="flex min-h-0 flex-1">
      <div class="flex min-w-0 flex-1 flex-col">
        <DataGrid
          v-if="t.cols.length" :label="`${t.schema}.${t.table} rows`" :columns="t.cols" :rows="t.rows" :selected="t.selected"
          :sort-column="sort?.column ?? null" :sort-descending="sort?.descending ?? false" @select="(r: number, c: number) => store.select(t, r, c)" @sort="(c: string) => store.sortBy(t, c)"
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

    <footer class="flex h-9 shrink-0 items-center gap-3 border-t border-border px-3 text-xs text-muted">
      <span v-if="t.rows.length" :data-testid="tid('sql-range')">Rows {{ fmt(first) }}–{{ fmt(last) }} of {{ total }}<template v-if="t.elapsedMs !== null"> · {{ t.elapsedMs }} ms</template></span>
      <span v-else-if="t.cols.length">0 rows</span>
      <Button v-if="t.exact === null && t.rows.length" variant="ghost" size="xs" @click="store.countExactly(t)">Count exactly</Button>
      <span class="ml-auto flex items-center gap-1">
        <IconButton label="Previous page" :disabled="t.page === 0 || t.loading" @click="store.prevPage(t)"><ChevronLeft /></IconButton>
        <span class="min-w-12 text-center">Page {{ t.page + 1 }}</span>
        <IconButton label="Next page" :disabled="!t.next || t.loading" @click="store.nextPage(t)"><ChevronRight /></IconButton>
      </span>
    </footer>
  </div>
</template>
