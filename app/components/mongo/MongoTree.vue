<script setup lang="ts">
// The tree of a MongoDB tab (S44): databases, opened lazily, and each one's collections and views.
import { ChevronDown, ChevronRight, Eye, SquareTerminal, Table2 } from '@lucide/vue'
import { Input } from '@/components/ui/input'

const props = defineProps<{ tabKey: string }>()
const store = useMongoStore(props.tabKey)
const rows = <T extends { name: string }>(c: T[] | null) => {
  const q = store.treeFilter.trim().toLowerCase()
  return (c ?? []).filter(o => !q || o.name.toLowerCase().includes(q))
}
function onKey(e: KeyboardEvent, open: () => void) {
  const el = e.currentTarget as HTMLElement
  const all = Array.from(el.closest('[role=tree]')?.querySelectorAll<HTMLElement>('[role=treeitem]') ?? [])
  const i = all.indexOf(el)
  if (e.key === 'Enter') { e.preventDefault(); open() }
  else if (e.key === 'ArrowDown') { e.preventDefault(); all[i + 1]?.focus() }
  else if (e.key === 'ArrowUp') { e.preventDefault(); all[i - 1]?.focus() }
  else if (e.key === 'Home') { e.preventDefault(); all[0]?.focus() }
  else if (e.key === 'End') { e.preventDefault(); all.at(-1)?.focus() }
}
function onDbKey(e: KeyboardEvent, d: (typeof store.dbs)[number]) {
  if (e.key === 'ArrowRight' && !d.open) { e.preventDefault(); void store.toggleDb(d, true) }
  else if (e.key === 'ArrowLeft' && d.open) { e.preventDefault(); void store.toggleDb(d, false) }
  else onKey(e, () => store.toggleDb(d))
}
</script>

<template>
  <nav class="flex w-64 shrink-0 flex-col border-r border-border" aria-label="Collections">
    <div class="shrink-0 p-2"><Input v-model="store.treeFilter" class="h-7 text-xs" placeholder="Find a collection" aria-label="Find a collection" autocomplete="off" spellcheck="false" /></div>
    <div role="tree" aria-label="Databases and collections" class="min-h-0 flex-1 overflow-auto px-1 pb-2 text-sm">
      <div v-for="d in store.dbs" :key="d.name" role="none">
        <div role="treeitem" tabindex="0" :aria-expanded="d.open" class="flex h-7 cursor-default items-center gap-1 rounded-md px-1.5 font-medium outline-none hover:bg-surface focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" :data-testid="tid(`mongo-db-${d.name}`)" @click="store.toggleDb(d)" @keydown="onDbKey($event, d)">
          <component :is="d.open ? ChevronDown : ChevronRight" class="size-3.5 text-muted" aria-hidden="true" /><span class="sr-only">Database </span>{{ d.name }}
        </div>
        <template v-if="d.open">
          <p v-if="d.loading" class="px-7 py-1 text-xs text-muted" aria-busy="true">Listing…</p>
          <p v-else-if="d.error" role="alert" class="px-7 py-1 text-xs text-danger">{{ d.error }}</p>
          <p v-else-if="d.colls && !d.colls.length" class="px-7 py-1 text-xs text-muted">This database has no collections yet.</p>
          <div
            v-for="c in rows(d.colls)" :key="c.name" role="treeitem" tabindex="0" :aria-selected="store.activeTab === `${d.name}.${c.name}`"
            class="flex h-7 cursor-default items-center gap-2 rounded-md pl-6 pr-2 outline-none hover:bg-surface focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent aria-selected:bg-surface-hover"
            :data-testid="tid(`mongo-coll-${c.name}`)" @click="store.openColl(d.name, c)" @keydown="onKey($event, () => store.openColl(d.name, c))"
          >
            <component :is="c.kind === 'view' ? Eye : Table2" class="size-3.5 shrink-0 text-muted" aria-hidden="true" />
            <span class="min-w-0 flex-1 truncate"><span class="sr-only">{{ c.kind }} </span>{{ c.name }}</span>
          </div>
        </template>
      </div>
      <p v-if="!store.dbs.length" class="px-3 py-2 text-xs text-muted" aria-busy="true">Reading the databases…</p>
    </div>
    <button type="button" class="flex h-9 shrink-0 cursor-default items-center gap-2 border-t border-border px-3 text-sm text-muted outline-none hover:bg-surface hover:text-text focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" :data-testid="tid('mongo-open-shell')" @click="store.openShell()"><SquareTerminal class="size-4" aria-hidden="true" />Shell</button>
  </nav>
</template>
