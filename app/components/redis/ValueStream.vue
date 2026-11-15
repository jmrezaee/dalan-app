<script setup lang="ts">
// S29: a stream's entries, newest first. Consumer groups are listed, not managed, in this version.
import type { RedisBytes, RedisStreamGroup } from '~/types/bindings'
import { bytesText } from '~/utils/redisBytes'

const props = defineProps<{ entries: { id: string, fields: { field: RedisBytes, value: RedisBytes }[] }[], groups: RedisStreamGroup[] }>()
const rows = computed(() => props.entries.map(e => [e.id, e.fields.map(f => `${bytesText(f.field)}=${bytesText(f.value)}`).join(' ')]))
</script>

<template>
  <ValueRows label="Entries" :columns="[{ label: 'ID', width: '170px' }, { label: 'Fields' }]" :rows="rows" />
  <p v-if="groups.length" class="shrink-0 border-t border-border px-3 py-2 text-xs text-muted">Groups: {{ groups.map(g => `${g.name} (${g.pending ?? 0} waiting)`).join(', ') }}.</p>
</template>
