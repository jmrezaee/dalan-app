<script setup lang="ts">
// The object tree of a SQL tab (S36): schemas, opened lazily, and each one's tables and views with the planner's row
// estimate. The search box filters what is already loaded. Enter or double-click opens a table.
import { Activity, ArrowLeftRight, ChevronDown, ChevronRight, Eye, Plus, Share2, Table2 } from '@lucide/vue'
import { Input } from '@/components/ui/input'
import type { SqlObject_Serialize as SqlObject } from '~/types/bindings'

const props = defineProps<{ tabKey: string }>()
const store = useSqlStore(props.tabKey)

const rows = (objs: SqlObject[] | null) => {
  const q = store.treeFilter.trim().toLowerCase()
  return (objs ?? []).filter(o => !q || o.name.toLowerCase().includes(q))
}
function compact(n: number | null) {
  if (n === null) return ''
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`
  if (n >= 1000) return `${(n / 1000).toFixed(n >= 10_000 ? 0 : 1).replace(/\.0$/, '')}k`
  return String(n)
}
const ICON = { table: Table2, view: Eye, matview: Eye, foreign: Table2, schema: Table2 } as const
const KIND_WORD = { table: 'table', view: 'view', matview: 'materialized view', foreign: 'foreign table', schema: 'schema' } as const

// ↑/↓ move between the tree's rows; Enter opens a table.
function onKey(e: KeyboardEvent, open: () => void) {
  const el = e.currentTarget as HTMLElement
  const all = Array.from(el.closest('[role=tree]')?.querySelectorAll<HTMLElement>('[role=treeitem]') ?? [])
  const i = all.indexOf(el)
  if (e.key === 'Enter') { e.preventDefault(); open() }
  else if (e.key === 'ArrowDown') { e.preventDefault(); all[i + 1]?.focus() }
  else if (e.key === 'Home') { e.preventDefault(); all[0]?.focus() }
  else if (e.key === 'End') { e.preventDefault(); all.at(-1)?.focus() }
  else if (e.key === 'ArrowUp') { e.preventDefault(); all[i - 1]?.focus() }
}
function onSchemaKey(e: KeyboardEvent, s: (typeof store.schemas)[number]) {
  // → opens a schema and ← closes it, like a tree.
  if (e.key === 'ArrowRight' && !s.open) { e.preventDefault(); void store.toggleSchema(s, true) }
  else if (e.key === 'ArrowLeft' && s.open) { e.preventDefault(); void store.toggleSchema(s, false) }
  else onKey(e, () => store.toggleSchema(s))
}
</script>

<template>
  <nav class="flex w-64 shrink-0 flex-col border-r border-border" aria-label="Objects">
    <div class="shrink-0 p-2"><Input v-model="store.treeFilter" class="h-7 text-xs" placeholder="Find a table" aria-label="Find a table" autocomplete="off" spellcheck="false" /></div>
    <div role="tree" aria-label="Schemas and tables" class="min-h-0 flex-1 overflow-auto px-1 pb-2 text-sm">
      <div v-for="s in store.schemas" :key="s.name" role="none">
        <div
          role="treeitem" tabindex="0" :aria-expanded="s.open" class="flex h-7 cursor-default items-center gap-1 rounded-md px-1.5 font-medium outline-none hover:bg-surface focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent"
          @click="store.toggleSchema(s)" @keydown="onSchemaKey($event, s)"
        >
          <component :is="s.open ? ChevronDown : ChevronRight" class="size-3.5 text-muted" aria-hidden="true" /><span class="sr-only">Schema </span>{{ s.name }}
        </div>
        <template v-if="s.open">
          <p v-if="s.loading" class="px-7 py-1 text-xs text-muted" aria-busy="true">Listing…</p>
          <p v-else-if="s.error" role="alert" class="px-7 py-1 text-xs text-danger">{{ s.error }}</p>
          <p v-else-if="s.objects && !s.objects.length" class="px-7 py-1 text-xs text-muted">This schema has no tables yet.</p>
          <div
            v-for="o in rows(s.objects)" :key="o.name" role="treeitem" tabindex="0" :aria-selected="store.activeTab === `${s.name}.${o.name}`"
            class="flex h-7 cursor-default items-center gap-2 rounded-md pl-6 pr-2 outline-none hover:bg-surface focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent aria-selected:bg-surface-hover"
            :data-testid="tid(`sql-object-${o.name}`)" @click="store.openTable(s.name, o)" @keydown="onKey($event, () => store.openTable(s.name, o))"
          >
            <component :is="ICON[o.kind]" class="size-3.5 shrink-0 text-muted" aria-hidden="true" />
            <span class="min-w-0 flex-1 truncate"><span class="sr-only">{{ KIND_WORD[o.kind] }} </span>{{ o.name }}</span>
            <span v-if="o.estimate !== null" class="text-2xs text-muted" :title="'About ' + o.estimate.toLocaleString('en-US') + ' rows (the planner\'s estimate)'">{{ compact(o.estimate) }}</span>
          </div>
        </template>
      </div>
      <p v-if="!store.schemas.length" class="px-3 py-2 text-xs text-muted" aria-busy="true">Reading the schemas…</p>
    </div>
    <button type="button" class="flex h-9 shrink-0 cursor-default items-center gap-2 border-t border-border px-3 text-sm text-muted outline-none hover:bg-surface hover:text-text focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" :data-testid="tid('sql-new-query')" @click="store.newQuery()"><Plus class="size-4" aria-hidden="true" />New query</button>
    <button type="button" class="flex h-9 shrink-0 cursor-default items-center gap-2 border-t border-border px-3 text-sm text-muted outline-none hover:bg-surface hover:text-text focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" :data-testid="tid('sql-new-diagram')" @click="store.openDiagram(store.current?.schema, store.current?.table)"><Share2 class="size-4" aria-hidden="true" />Diagram</button>
  <div class="flex shrink-0 border-t border-border">
      <button type="button" class="flex h-9 flex-1 cursor-default items-center gap-2 px-3 text-sm text-muted outline-none hover:bg-surface hover:text-text focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" :data-testid="tid('sql-open-activity')" @click="store.openSpecial('activity')"><Activity class="size-4" aria-hidden="true" />Activity</button>
      <button v-if="!store.isMy" type="button" class="flex h-9 flex-1 cursor-default items-center gap-2 px-3 text-sm text-muted outline-none hover:bg-surface hover:text-text focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" :data-testid="tid('sql-open-transfer')" @click="store.openSpecial('transfer')"><ArrowLeftRight class="size-4" aria-hidden="true" />Transfer</button>
    </div>
  </nav>
</template>
