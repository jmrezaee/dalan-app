<script setup lang="ts">
// S29: a list's items, by position from 0.
import type { RedisBytes } from '~/types/bindings'

const props = defineProps<{ items: RedisBytes[] }>()
const rows = computed(() => props.items.map((v, i) => [String(i), v]))
</script>

<template>
  <ValueRows label="Items" :columns="[{ label: '#', width: '72px' }, { label: 'Value' }]" :rows="rows" />
</template>
