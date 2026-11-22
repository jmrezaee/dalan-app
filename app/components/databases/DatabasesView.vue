<script setup lang="ts">
// S34: the saved databases of every engine, with engine filter chips. Opening a Redis one adds a Redis tab.
import { Database, Pencil, Plug, Plus, Trash2 } from '@lucide/vue'
import type { MenuItem } from '@/components/common/ItemMenuList.vue'
import { ContextMenu, ContextMenuContent, ContextMenuTrigger } from '@/components/ui/context-menu'
import { buttonVariants } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { DbRow } from '~/stores/databases'
import { ENGINES } from '~/utils/engines'

const databases = useDatabasesStore()
const redisDatabases = useRedisDatabasesStore()
const sessions = useSessionsStore()
const hosts = useHostsStore()

const FILTERS = [
  { id: null, label: 'All', engines: null },
  { id: 'redis', label: 'Redis', engines: ['redis'] },
  { id: 'postgres', label: 'PostgreSQL', engines: ['postgres'] },
  { id: 'mysql', label: 'MySQL / MariaDB', engines: ['mysql', 'mariadb'] },
  { id: 'mongodb', label: 'MongoDB', engines: ['mongodb'] },
] as const
const countOf = (engines: readonly string[] | null) => (engines ? databases.rows.filter(d => engines.includes(d.engine)).length : databases.rows.length)

const shown = computed(() => {
  const q = redisDatabases.filter.trim().toLowerCase()
  const f = FILTERS.find(x => x.id === databases.engineFilter)
  return databases.rows.filter(d =>
    (!f?.engines || (f.engines as readonly string[]).includes(d.engine))
    && (!q || `${d.name} ${d.address} ${d.host_label ?? ''} ${d.environment} ${ENGINES[d.engine].label}`.toLowerCase().includes(q)))
})

/** "bastion → db-primary" for a database behind a jump chain, "Direct" otherwise. */
function routeText(d: DbRow) {
  if (d.route === 'direct') return 'Direct'
  const host = hosts.hosts.find(h => h.id === d.host_id)
  const via = (host?.effective_jump_host_ids ?? []).map(id => hosts.hosts.find(h => h.id === id)?.label).filter(Boolean)
  return [...via, d.host_label ?? 'a host'].join(' → ')
}
const REDIS_KIND = { single: 'Single server', cluster: 'Cluster', sentinel: 'Sentinel' } as const
function kindText(d: DbRow) {
  if (d.engine === 'redis') return REDIS_KIND[redisDatabases.dbs.find(r => r.id === d.id)?.kind ?? 'single']
  if (d.engine === 'mongodb') return d.options.srv ? 'SRV' : d.options.replica_set ? 'Replica set' : 'Single server'
  return 'Single server'
}
const addressText = (d: DbRow) => (d.engine === 'mongodb' || d.port == null || d.address.includes(',') ? d.address : `${d.address}:${d.port}`) + (d.engine !== 'redis' && d.default_database ? ` · ${d.default_database}` : '')
const MODE = {
  read_only: { label: 'Read-only', cls: 'border-border text-muted' },
  confirm: { label: 'Confirm writes', cls: 'border-accent/40 text-accent' },
  open: { label: 'Open', cls: 'border-warning/40 text-warning' },
} as const
const ENV_BAR = { production: 'bg-warning', staging: 'bg-border-input', none: 'bg-transparent' } as const

const notice = ref('')
function open(d: DbRow) {
  notice.value = ''
  if (d.engine === 'redis') {
    const r = redisDatabases.dbs.find(x => x.id === d.id)
    if (r) sessions.openRedis(r)
    return
  }
  if (d.engine === 'postgres') {
    sessions.openSql(d)
    return
  }
  notice.value = `${d.name} is saved. Browsing ${ENGINES[d.engine].label} databases arrives in a later update.`
}
function edit(d: DbRow) {
  if (d.engine === 'redis') {
    const r = redisDatabases.dbs.find(x => x.id === d.id)
    if (r) redisDatabases.editing = r
    return
  }
  databases.pasted = null
  databases.editing = d
}

const toDelete = ref<DbRow | null>(null)
const confirmOpen = ref(false)
const deleteError = ref('')
function askDelete(d: DbRow) {
  toDelete.value = d
  deleteError.value = ''
  confirmOpen.value = true
}
async function confirmDelete() {
  const d = toDelete.value
  if (!d) return
  // A tab on this database closes with it.
  const tab = sessions.sessions.find(s => (s.kind === 'redis' || s.kind === 'sql') && s.dbId === d.id)
  if (tab) sessions.close(tab)
  deleteError.value = await databases.remove(d)
  if (!deleteError.value) {
    confirmOpen.value = false
    if (d.engine === 'redis') await redisDatabases.refresh()
  }
}
const menuFor = (d: DbRow): MenuItem[] => [
  { label: 'Open', icon: Plug, onSelect: () => open(d) },
  { label: 'Edit', icon: Pencil, onSelect: () => edit(d) },
  { label: 'Delete', icon: Trash2, danger: true, separator: true, onSelect: () => askDelete(d) },
]

// ↑/↓ move between rows, like a list.
function onRowKey(e: KeyboardEvent, d: DbRow) {
  const row = e.currentTarget as HTMLElement
  if (e.key === 'Enter') { e.preventDefault(); open(d) }
  else if (e.key === 'ArrowDown') { e.preventDefault(); (row.nextElementSibling as HTMLElement | null)?.focus() }
  else if (e.key === 'ArrowUp') { e.preventDefault(); (row.previousElementSibling as HTMLElement | null)?.focus() }
}
onMounted(() => { if (!databases.loaded) databases.refresh() })
</script>

<template>
  <PageToolbar title="Databases">
    <span class="mr-auto" />
    <SearchInput v-if="databases.rows.length" v-model="redisDatabases.filter" placeholder="Filter databases" class="w-[220px]" />
    <Button v-if="databases.rows.length" @click="databases.openPicker()"><Plus />New database</Button>
  </PageToolbar>

  <div class="flex-1 overflow-auto px-3 py-2">
    <div v-if="!databases.loaded" class="flex flex-col gap-2 p-3" aria-busy="true"><div v-for="i in 3" :key="i" class="h-9 animate-pulse rounded-md bg-surface" /></div>
    <EmptyState v-else-if="!databases.rows.length" :icon="Database" title="No databases yet" text="Add PostgreSQL, MySQL, MariaDB, MongoDB or Redis. Browse, query and change data from one place.">
      <Button @click="databases.openPicker()"><Plus />New database</Button>
    </EmptyState>
    <template v-else>
      <div class="flex gap-1 px-0 pb-2" role="group" aria-label="Filter by engine">
        <Button v-for="f in FILTERS" :key="String(f.id)" size="sm" :variant="databases.engineFilter === f.id ? 'secondary' : 'ghost'" :aria-pressed="databases.engineFilter === f.id" :class="databases.engineFilter === f.id ? '' : 'text-muted'" @click="databases.engineFilter = f.id">{{ f.label }} {{ countOf(f.engines) }}</Button>
      </div>
      <table class="w-full border-collapse text-sm" aria-label="Databases">
        <thead>
          <tr class="text-left text-xs text-muted">
            <th scope="col" class="h-8 border-b border-border px-3 font-medium">Name</th>
            <th scope="col" class="h-8 border-b border-border px-3 font-medium">Address</th>
            <th scope="col" class="h-8 border-b border-border px-3 font-medium">Route</th>
            <th scope="col" class="h-8 border-b border-border px-3 font-medium">Kind</th>
            <th scope="col" class="h-8 border-b border-border px-3 font-medium">Writes</th>
          </tr>
        </thead>
        <tbody>
          <ContextMenu v-for="d in shown" :key="d.id">
            <ContextMenuTrigger as-child>
              <tr tabindex="0" class="cursor-default whitespace-nowrap border-b border-surface-raised outline-none hover:bg-surface focus-visible:bg-surface focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" :data-testid="tid(`database-row-${d.name}`)" @dblclick="open(d)" @keydown="onRowKey($event, d)">
                <td class="h-11 px-3">
                  <span class="inline-flex items-center gap-2.5 font-medium">
                    <span :class="cn('h-5 w-[3px] rounded-sm', ENV_BAR[d.environment])" aria-hidden="true" /><EngineBadge :engine="d.engine" />{{ d.name }}
                    <span v-if="d.environment !== 'none'" :class="cn('text-xs font-normal', d.environment === 'production' ? 'text-warning' : 'text-muted')">{{ d.environment }}</span>
                  </span>
                </td>
                <td class="h-11 px-3 font-mono text-xs text-muted">{{ addressText(d) }}</td>
                <td class="h-11 px-3 text-muted">{{ routeText(d) }}</td>
                <td class="h-11 px-3 text-muted">{{ kindText(d) }}</td>
                <td class="h-11 px-3"><span :class="cn('inline-flex h-5 items-center rounded-full border px-2 text-2xs font-medium', MODE[d.write_mode].cls)">{{ MODE[d.write_mode].label }}</span></td>
              </tr>
            </ContextMenuTrigger>
            <ContextMenuContent><ItemMenuList :items="menuFor(d)" mode="context" /></ContextMenuContent>
          </ContextMenu>
        </tbody>
      </table>
      <p v-if="!shown.length" class="px-3 pt-4 text-sm text-muted">No database matches {{ redisDatabases.filter.trim() || 'this filter' }}.</p>
      <p v-if="notice" role="status" class="px-3 pt-4 text-sm text-muted">{{ notice }}</p>
      <p class="max-w-[640px] px-3 pt-4 text-xs text-muted">One list for every engine. Each database keeps its own route, sign-in, environment and write mode. Saved queries live in the vault too, so they follow the database, not a file on someone's laptop.</p>
    </template>
  </div>

  <AlertDialog v-model:open="confirmOpen">
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>Delete {{ toDelete?.name }}?</AlertDialogTitle>
        <AlertDialogDescription>Dalan forgets this database and its saved password. Nothing changes on the server.</AlertDialogDescription>
      </AlertDialogHeader>
      <p v-if="deleteError" role="alert" class="text-sm text-danger">{{ deleteError }}</p>
      <AlertDialogFooter>
        <AlertDialogCancel>Cancel</AlertDialogCancel>
        <Button :class="buttonVariants({ variant: 'destructive' })" @click="confirmDelete">Delete database</Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
