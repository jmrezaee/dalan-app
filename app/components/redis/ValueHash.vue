<script setup lang="ts">
// S28: a hash's fields. The filter narrows what has been loaded so far (it does not ask the server).
import { Input } from '@/components/ui/input'
import type { RedisBytes } from '~/types/bindings'
import { bytesText } from '~/utils/redisBytes'

const props = defineProps<{ fields: { field: RedisBytes, value: RedisBytes }[] }>()
const filter = ref('')
const rows = computed(() => {
  const q = filter.value.trim().toLowerCase()
  const list = q ? props.fields.filter(f => bytesText(f.field).toLowerCase().includes(q) || bytesText(f.value).toLowerCase().includes(q)) : props.fields
  return list.map(f => [f.field, f.value])
})
</script>

<template>
  <div class="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
    <Input v-model="filter" aria-label="Filter fields" class="h-7 w-[240px] text-xs" placeholder="Filter fields" autocomplete="off" spellcheck="false" />
    <span v-if="filter" class="text-xs text-muted">{{ rows.length }} of {{ fields.length }} loaded</span>
  </div>
  <ValueRows label="Fields" :columns="[{ label: 'Field', width: 'minmax(120px, 30%)' }, { label: 'Value' }]" :rows="rows" />
</template>
