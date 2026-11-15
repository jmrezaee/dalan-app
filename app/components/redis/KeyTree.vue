<script setup lang="ts">
// S28, left side: the keys scanned so far, grouped by the database's separator. SCAN only, never KEYS.
import { ChevronRight, Plus, RefreshCw } from '@lucide/vue'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { cn } from '@/lib/utils'
import { count, ttlText } from '~/utils/redisBytes'
import { treeRows } from '~/utils/redisTree'
import type { TreeRow } from '~/utils/redisTree'

const props = defineProps<{ tabKey: string, separator: string, writable?: boolean }>()
defineEmits<{ newKey: [] }>()
const store = useRedisStore(props.tabKey)

const TYPES = [{ value: 'all', label: 'All types' }, { value: 'string', label: 'String' }, { value: 'hash', label: 'Hash' }, { value: 'list', label: 'List' }, { value: 'set', label: 'Set' }, { value: 'zset', label: 'Sorted set' }, { value: 'stream', label: 'Stream' }, { value: 'json', label: 'JSON' }]
const draft = ref(store.pattern)
function applyPattern() {
  store.pattern = draft.value.trim()
  void store.scan()
}
function setType(v: unknown) {
  store.typeFilter = v === 'all' ? null : String(v)
  void store.scan()
}

const rows = computed(() => treeRows(store.keyRows, props.separator, store.openFolders))
const ROW = 28
const { scroller, onScroll, from, to, padTop, padBottom, reveal } = useVirtualRows(computed(() => rows.value.length), ROW)
const shown = computed(() => rows.value.slice(from.value, to.value))

// ---- Keyboard: the tree is one Tab stop; arrows move a focused row (aria-activedescendant).
const focused = ref(0)
watch(rows, (r) => { if (focused.value >= r.length) focused.value = Math.max(0, r.length - 1) })
const rowId = (i: number) => `${props.tabKey}-key-${i}`
function activate(row: TreeRow) {
  if (row.kind === 'folder') store.toggleFolder(row.path)
  else void store.openKey(row.row.key)
}
function move(to: number) {
  focused.value = Math.min(rows.value.length - 1, Math.max(0, to))
  reveal(focused.value)
}
function onKey(e: KeyboardEvent) {
  const row = rows.value[focused.value]
  if (!row) return
  switch (e.key) {
    case 'ArrowDown': move(focused.value + 1); break
    case 'ArrowUp': move(focused.value - 1); break
    case 'Home': move(0); break
    case 'End': move(rows.value.length - 1); break
    case 'Enter': case ' ': activate(row); break
    case 'ArrowRight':
      if (row.kind === 'folder' && !row.open) store.toggleFolder(row.path)
      else if (row.kind === 'folder') move(focused.value + 1)
      break
    case 'ArrowLeft':
      if (row.kind === 'folder' && row.open) store.toggleFolder(row.path)
      else {
        // To the folder this row sits in: the nearest row above with a smaller depth.
        for (let i = focused.value - 1; i >= 0; i--) {
          if (rows.value[i]!.depth < row.depth) { move(i); break }
        }
      }
      break
    default: return
  }
  e.preventDefault()
}

const total = computed(() => store.keys[store.selected] ?? 0)
const ttl = (ms: number | null | undefined) => (ms == null ? 'no expiry' : ttlText(ms, 1))
</script>

<template>
  <section aria-label="Keys" class="flex w-[320px] shrink-0 flex-col border-r border-border bg-surface">
    <form class="flex h-11 shrink-0 items-center gap-1.5 border-b border-border px-2" @submit.prevent="applyPattern">
      <Input v-model="draft" aria-label="Key pattern" class="h-7 min-w-0 flex-1 font-mono text-xs" placeholder="user:*" autocomplete="off" spellcheck="false" />
      <Select :model-value="store.typeFilter ?? 'all'" @update:model-value="setType">
        <SelectTrigger class="h-7 w-auto gap-1 px-2 text-xs" aria-label="Filter by type"><SelectValue /></SelectTrigger>
        <SelectContent><SelectItem v-for="t in TYPES" :key="t.value" :value="t.value" class="text-xs">{{ t.label }}</SelectItem></SelectContent>
      </Select>
      <IconButton label="Refresh keys" @click="store.scan()"><RefreshCw /></IconButton>
      <IconButton v-if="writable" label="New key" @click="$emit('newKey')"><Plus /></IconButton>
    </form>

    <div
      ref="scroller"
      role="tree"
      aria-label="Keys"
      tabindex="0"
      :aria-activedescendant="rows.length ? rowId(focused) : undefined"
      class="min-h-0 flex-1 overflow-auto py-1 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent"
      :data-testid="tid('redis-tree')"
      @scroll="onScroll"
      @keydown="onKey"
    >
      <div :style="{ height: `${padTop}px` }" aria-hidden="true" />
      <div
        v-for="(r, i) in shown"
        :id="rowId(from + i)"
        :key="r.kind === 'folder' ? `f:${r.path}` : r.id"
        role="treeitem"
        :aria-level="r.depth + 1"
        :aria-expanded="r.kind === 'folder' ? r.open : undefined"
        :aria-selected="r.kind === 'key' ? r.id === store.selectedId : undefined"
        :class="cn(
          'flex h-7 cursor-default items-center gap-1.5 pr-2 text-sm',
          r.kind === 'key' && r.id === store.selectedId ? 'bg-accent/10' : 'hover:bg-surface-hover',
          from + i === focused && 'shadow-[inset_2px_0_0_var(--accent)]',
        )"
        :style="{ paddingLeft: `${8 + r.depth * 14}px` }"
        @click="focused = from + i; activate(r)"
      >
        <template v-if="r.kind === 'folder'">
          <ChevronRight :class="cn('size-3.5 shrink-0 text-muted transition-transform', r.open && 'rotate-90')" aria-hidden="true" />
          <span class="min-w-0 flex-1 truncate font-mono text-xs">{{ r.name }}</span>
          <span class="text-2xs text-muted">{{ count(r.count) }}</span>
        </template>
        <template v-else>
          <RedisTypeChip :type="r.row.key_type" />
          <span class="min-w-0 flex-1 truncate font-mono text-xs" :title="r.name">{{ r.name }}</span>
          <span class="shrink-0 text-2xs text-muted">{{ ttl(r.row.ttl_ms) }}</span>
        </template>
      </div>
      <div :style="{ height: `${padBottom}px` }" aria-hidden="true" />

      <p v-if="store.scanError" role="alert" class="px-3 py-2 text-xs text-danger">{{ store.scanError }} <button type="button" class="underline" @click="store.scanMore()">Try again</button></p>
      <p v-else-if="!rows.length && !store.scanning && store.scanDone" class="px-3 py-2 text-xs text-muted">{{ store.pattern || store.typeFilter ? `No keys match ${store.pattern || 'that type'}.` : 'This database has no keys.' }}</p>
      <p v-else-if="!rows.length && !store.scanning && (store.pattern || store.typeFilter)" class="px-3 py-2 text-xs text-muted">No keys match {{ store.pattern || 'that type' }} in what's been scanned so far. Scan more?</p>
    </div>

    <div class="flex h-9 shrink-0 items-center gap-2 border-t border-border px-3 text-xs text-muted" aria-live="polite">
      <span v-if="store.scanning" class="flex items-center gap-2"><span class="size-3 animate-spin rounded-full border-2 border-accent border-r-transparent" aria-hidden="true" />Scanning…</span>
      <span v-else-if="store.scanDone">All {{ count(total) }} keys scanned</span>
      <span v-else>Scanned {{ count(Math.min(store.scanned, total || store.scanned)) }} of {{ count(total) }} keys</span>
      <Button v-if="!store.scanDone && !store.scanning" variant="ghost" size="sm" class="ml-auto text-accent" @click="store.scanMore()">Scan more</Button>
    </div>
  </section>
</template>
