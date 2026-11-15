<script setup lang="ts">
// S28: a hash's fields. The filter narrows what has been loaded so far (it does not ask the server).
// Writable (R3): add a field, change a value, remove a field. Each goes through the write gate in Rust.
import { Plus } from '@lucide/vue'
import { toast } from 'vue-sonner'
import { Input } from '@/components/ui/input'
import type { RedisBytes } from '~/types/bindings'
import { bytesText, textBytes } from '~/utils/redisBytes'

const props = defineProps<{ fields: { field: RedisBytes, value: RedisBytes }[], tabKey: string, writable: boolean, saveLabel: string }>()
const store = useRedisStore(props.tabKey)
const filter = ref('')
const list = computed(() => {
  const q = filter.value.trim().toLowerCase()
  return q ? props.fields.filter(f => bytesText(f.field).toLowerCase().includes(q) || bytesText(f.value).toLowerCase().includes(q)) : props.fields
})
const rows = computed(() => list.value.map(f => [f.field, f.value]))
const table = ref<{ startAdd: (fixed: (string | null)[]) => void } | null>(null)

function save(i: number, values: string[]) {
  const key = store.selectedKey
  if (!key) return
  if (i < 0) {
    if (!values[0]) return void toast.error('Give the field a name.')
    const exists = props.fields.some(f => bytesText(f.field) === values[0])
    return void store.write({ op: 'hash_set', key, field: textBytes(values[0]), value: textBytes(values[1] ?? '') }, exists ? { title: 'Change this field?', action: 'Change' } : { title: 'Add this field?', action: 'Add field' })
  }
  const f = list.value[i]
  if (f) void store.write({ op: 'hash_set', key, field: f.field, value: textBytes(values[1] ?? '') }, { title: 'Change this field?', action: 'Change' })
}
function remove(i: number) {
  const f = list.value[i]
  if (f && store.selectedKey) void store.write({ op: 'hash_remove', key: store.selectedKey, field: f.field }, { title: 'Remove this field?', action: 'Remove field', danger: true })
}
</script>

<template>
  <div class="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
    <Input v-model="filter" aria-label="Filter fields" class="h-7 w-[240px] text-xs" placeholder="Filter fields" autocomplete="off" spellcheck="false" />
    <span v-if="filter" class="text-xs text-muted">{{ rows.length }} of {{ fields.length }} loaded</span>
    <Button v-if="writable" variant="secondary" size="sm" class="ml-auto" @click="table?.startAdd([null, null])"><Plus />Add field</Button>
  </div>
  <ValueRows ref="table" label="Fields" item="field" :columns="[{ label: 'Field', width: 'minmax(120px, 30%)' }, { label: 'Value' }]" :rows="rows" :edit="writable ? [1] : undefined" :removable="writable" :save-label="saveLabel" @save="save" @remove="remove" />
</template>
