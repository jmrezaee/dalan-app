<script setup lang="ts">
// The Saved queries list: SQL, pipelines and filters kept in the vault, for every database of their engine.
import { Star, Trash2 } from '@lucide/vue'
import { ENGINES } from '~/utils/engines'
import type { SavedQueryRow } from '~/stores/databases'

const databases = useDatabasesStore()
const error = ref('')
onMounted(() => { if (!databases.savedLoaded) databases.refreshSaved() })
async function remove(q: SavedQueryRow) {
  error.value = await databases.removeSaved(q)
}
const KIND = { sql: 'SQL', pipeline: 'Pipeline', filter: 'Filter', command: 'Command' } as const
</script>

<template>
  <PageToolbar title="Saved queries" />
  <div class="flex-1 overflow-auto px-3 py-2">
    <div v-if="!databases.savedLoaded" class="flex flex-col gap-2 p-3" aria-busy="true"><div v-for="i in 3" :key="i" class="h-9 animate-pulse rounded-md bg-surface" /></div>
    <EmptyState v-else-if="!databases.saved.length" :icon="Star" title="No saved queries yet" text="Save a query with ⌘S and it shows up here for every database of that engine." />
    <template v-else>
      <table class="w-full border-collapse text-sm" aria-label="Saved queries">
        <thead>
          <tr class="text-left text-xs text-muted">
            <th scope="col" class="h-8 border-b border-border px-3 font-medium">Name</th>
            <th scope="col" class="h-8 border-b border-border px-3 font-medium">Kind</th>
            <th scope="col" class="h-8 border-b border-border px-3 font-medium">Database</th>
            <th scope="col" class="h-8 border-b border-border px-3 font-medium">Folder</th>
            <th scope="col" class="h-8 w-10 border-b border-border px-3"><span class="sr-only">Actions</span></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="q in databases.saved" :key="q.id" class="border-b border-surface-raised hover:bg-surface" :data-testid="tid(`saved-row-${q.name}`)">
            <td class="h-11 px-3"><span class="inline-flex items-center gap-2.5 font-medium"><EngineBadge :engine="q.engine" />{{ q.name }}</span></td>
            <td class="h-11 px-3 text-muted">{{ KIND[q.kind] }}</td>
            <td class="h-11 px-3 text-muted">{{ q.database_name ?? `Any ${ENGINES[q.engine].label} database` }}</td>
            <td class="h-11 px-3 text-muted">{{ q.folder ?? '' }}</td>
            <td class="h-11 px-3"><IconButton :label="`Delete ${q.name}`" @click="remove(q)"><Trash2 /></IconButton></td>
          </tr>
        </tbody>
      </table>
      <p v-if="error" role="alert" class="px-3 pt-3 text-sm text-danger">{{ error }}</p>
    </template>
  </div>
</template>
