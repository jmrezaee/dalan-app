<script setup lang="ts">
// S26: the saved Redis databases. Opening one adds a Redis tab.
import { Database, Pencil, Plug, Plus, Trash2 } from '@lucide/vue'
import type { MenuItem } from '@/components/common/ItemMenuList.vue'
import { ContextMenu, ContextMenuContent, ContextMenuTrigger } from '@/components/ui/context-menu'
import { buttonVariants } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { SavedDatabase } from '~/stores/redisDatabases'

const databases = useRedisDatabasesStore()
const sessions = useSessionsStore()
const hosts = useHostsStore()

const shown = computed(() => {
  const q = databases.filter.trim().toLowerCase()
  return q ? databases.dbs.filter(d => `${d.name} ${d.address} ${d.host_label ?? ''} ${d.environment}`.toLowerCase().includes(q)) : databases.dbs
})

/** "bastion → db-primary" for a database behind a jump chain, "Direct" otherwise. */
function routeText(d: SavedDatabase) {
  if (d.route === 'direct') return 'Direct'
  const host = hosts.hosts.find(h => h.id === d.host_id)
  const via = (host?.effective_jump_host_ids ?? []).map(id => hosts.hosts.find(h => h.id === id)?.label).filter(Boolean)
  return [...via, d.host_label ?? 'a host'].join(' → ')
}
const kindText = { single: 'Single server', cluster: 'Cluster', sentinel: 'Sentinel' } as const
const MODE = {
  read_only: { label: 'Read-only', cls: 'border-border text-muted' },
  confirm: { label: 'Confirm writes', cls: 'border-accent/40 text-accent' },
  open: { label: 'Open', cls: 'border-warning/40 text-warning' },
} as const
const ENV_BAR = { production: 'bg-warning', staging: 'bg-border-input', none: 'bg-transparent' } as const

const toDelete = ref<SavedDatabase | null>(null)
const confirmOpen = ref(false)
const deleteError = ref('')
function askDelete(d: SavedDatabase) {
  toDelete.value = d
  deleteError.value = ''
  confirmOpen.value = true
}
async function confirmDelete() {
  const d = toDelete.value
  if (!d) return
  // A tab on this database closes with it.
  const tab = sessions.sessions.find(s => s.kind === 'redis' && s.dbId === d.id)
  if (tab) sessions.close(tab)
  deleteError.value = await databases.remove(d)
  if (!deleteError.value) confirmOpen.value = false
}
const menuFor = (d: SavedDatabase): MenuItem[] => [
  { label: 'Open', icon: Plug, onSelect: () => sessions.openRedis(d) },
  { label: 'Edit', icon: Pencil, onSelect: () => { databases.editing = d } },
  { label: 'Delete', icon: Trash2, danger: true, separator: true, onSelect: () => askDelete(d) },
]

// ↑/↓ move between rows, like a list.
function onRowKey(e: KeyboardEvent, d: SavedDatabase) {
  const row = e.currentTarget as HTMLElement
  if (e.key === 'Enter') { e.preventDefault(); sessions.openRedis(d) }
  else if (e.key === 'ArrowDown') { e.preventDefault(); (row.nextElementSibling as HTMLElement | null)?.focus() }
  else if (e.key === 'ArrowUp') { e.preventDefault(); (row.previousElementSibling as HTMLElement | null)?.focus() }
}
onMounted(() => { if (!databases.loaded) databases.refresh() })
</script>

<template>
  <PageToolbar title="Databases">
    <span class="mr-auto text-xs text-muted">Redis</span>
    <SearchInput v-if="databases.dbs.length" v-model="databases.filter" placeholder="Filter databases" class="w-[220px]" />
    <Button v-if="databases.dbs.length" @click="databases.openNew()"><Plus />New database</Button>
  </PageToolbar>

  <div class="flex-1 overflow-auto px-3 py-2">
    <div v-if="!databases.loaded" class="flex flex-col gap-2 p-3" aria-busy="true"><div v-for="i in 3" :key="i" class="h-9 animate-pulse rounded-md bg-surface" /></div>
    <EmptyState v-else-if="!databases.dbs.length" :icon="Database" title="No databases yet" text="Add one to browse its keys, run commands and watch channels.">
      <Button @click="databases.openNew()"><Plus />New database</Button>
    </EmptyState>
    <template v-else>
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
              <tr tabindex="0" class="cursor-default whitespace-nowrap border-b border-surface-raised outline-none hover:bg-surface focus-visible:bg-surface focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" :data-testid="tid(`database-row-${d.name}`)" @dblclick="sessions.openRedis(d)" @keydown="onRowKey($event, d)">
                <td class="h-11 px-3">
                  <span class="inline-flex items-center gap-2.5 font-medium">
                    <span :class="cn('h-5 w-[3px] rounded-sm', ENV_BAR[d.environment])" aria-hidden="true" />{{ d.name }}
                    <span v-if="d.environment !== 'none'" :class="cn('text-xs font-normal', d.environment === 'production' ? 'text-warning' : 'text-muted')">{{ d.environment }}</span>
                  </span>
                </td>
                <td class="h-11 px-3 font-mono text-xs text-muted">{{ d.address }}:{{ d.port }}</td>
                <td class="h-11 px-3 text-muted">{{ routeText(d) }}</td>
                <td class="h-11 px-3 text-muted">{{ kindText[d.kind] }}</td>
                <td class="h-11 px-3"><span :class="cn('inline-flex h-5 items-center rounded-full border px-2 text-2xs font-medium', MODE[d.write_mode].cls)">{{ MODE[d.write_mode].label }}</span></td>
              </tr>
            </ContextMenuTrigger>
            <ContextMenuContent><ItemMenuList :items="menuFor(d)" mode="context" /></ContextMenuContent>
          </ContextMenu>
        </tbody>
      </table>
      <p v-if="!shown.length" class="px-3 pt-4 text-sm text-muted">No database matches {{ databases.filter }}.</p>
      <p class="max-w-[520px] px-3 pt-4 text-xs text-muted">A database can be reached directly, or through any saved host. Passwords are kept in the vault, like your SSH passwords.</p>
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
