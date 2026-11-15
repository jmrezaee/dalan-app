<script setup lang="ts">
// S29: a list's items, by position from 0. Writable (R3): add at either end, change an item, remove by position.
// Rust checks that the position still holds the item shown here before it changes or removes it.
import { Plus } from '@lucide/vue'
import type { RedisBytes } from '~/types/bindings'
import { textBytes } from '~/utils/redisBytes'

const props = defineProps<{ items: RedisBytes[], tabKey: string, writable: boolean, saveLabel: string }>()
const store = useRedisStore(props.tabKey)
const rows = computed(() => props.items.map((v, i) => [String(i), v]))
const table = ref<{ startAdd: (fixed: (string | null)[]) => void } | null>(null)

function save(i: number, values: string[]) {
  const key = store.selectedKey
  if (!key) return
  const value = textBytes(values[1] ?? '')
  if (i < 0) return void store.write({ op: 'list_push', key, value, start: values[0] === 'start' }, { title: 'Add this item?', action: 'Add item' })
  const expect = props.items[i]
  if (expect) void store.write({ op: 'list_set', key, index: i, expect, value }, { title: 'Change this item?', action: 'Change' })
}
function remove(i: number) {
  const expect = props.items[i]
  if (expect && store.selectedKey) void store.write({ op: 'list_remove', key: store.selectedKey, index: i, expect }, { title: 'Remove this item?', action: 'Remove item', danger: true })
}
</script>

<template>
  <div v-if="writable" class="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
    <span class="text-xs text-muted">Add to the start or the end. Remove by position.</span>
    <Button variant="secondary" size="sm" class="ml-auto" aria-label="Add at the start" @click="table?.startAdd(['start', null])"><Plus />Start</Button>
    <Button variant="secondary" size="sm" aria-label="Add at the end" @click="table?.startAdd(['end', null])"><Plus />End</Button>
  </div>
  <ValueRows ref="table" label="Items" item="item" :columns="[{ label: '#', width: '72px' }, { label: 'Value' }]" :rows="rows" :edit="writable ? [1] : undefined" :removable="writable" :save-label="saveLabel" @save="save" @remove="remove" />
</template>
