<script setup lang="ts">
// S29: a stream's entries, newest first. Consumer groups are listed, not managed, in this version.
// Writable (R3): add an entry, delete an entry (an entry can't be changed: that is what a stream is).
import { Plus } from '@lucide/vue'
import { toast } from 'vue-sonner'
import type { RedisBytes, RedisStreamGroup } from '~/types/bindings'
import { bytesText, textBytes } from '~/utils/redisBytes'

const props = defineProps<{ entries: { id: string, fields: { field: RedisBytes, value: RedisBytes }[] }[], groups: RedisStreamGroup[], tabKey: string, writable: boolean, saveLabel: string }>()
const store = useRedisStore(props.tabKey)
const rows = computed(() => props.entries.map(e => [e.id, e.fields.map(f => `${bytesText(f.field)}=${bytesText(f.value)}`).join(' ')]))
const table = ref<{ startAdd: (fixed: (string | null)[], hints?: string[]) => void } | null>(null)

function save(_i: number, values: string[]) {
  // ponytail: fields are typed as "name=value name=value", so a value with a space needs the Console (R4).
  const parts = (values[1] ?? '').trim().split(/\s+/).filter(Boolean)
  if (!parts.length || parts.some(p => p.indexOf('=') < 1)) return void toast.error('Write the fields as name=value, with a space between them.')
  const fields = parts.map(p => ({ field: textBytes(p.slice(0, p.indexOf('='))), value: textBytes(p.slice(p.indexOf('=') + 1)) }))
  if (store.selectedKey) void store.write({ op: 'stream_add', key: store.selectedKey, id: (values[0] ?? '').trim() || null, fields }, { title: 'Add this entry?', action: 'Add entry' })
}
function remove(i: number) {
  const e = props.entries[i]
  if (e && store.selectedKey) void store.write({ op: 'stream_delete', key: store.selectedKey, id: e.id }, { title: 'Delete this entry?', action: 'Delete entry', danger: true })
}
</script>

<template>
  <div v-if="writable" class="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
    <Button variant="secondary" size="sm" class="ml-auto" @click="table?.startAdd([null, null], ['ID (empty: the server chooses)', 'order=5512 status=paid'])"><Plus />Add entry</Button>
  </div>
  <ValueRows ref="table" label="Entries" item="entry" :columns="[{ label: 'ID', width: '170px' }, { label: 'Fields' }]" :rows="rows" :removable="writable" :save-label="saveLabel" @save="save" @remove="remove" />
  <p v-if="groups.length" class="shrink-0 border-t border-border px-3 py-2 text-xs text-muted">Groups: {{ groups.map(g => `${g.name} (${g.pending ?? 0} waiting)`).join(', ') }}.</p>
</template>
