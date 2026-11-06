<script setup lang="ts">
// Search box plus an A–Z / Z–A switch, the same on every section list.
import { ArrowDownAZ, ArrowUpZA } from '@lucide/vue'

defineProps<{ placeholder: string, count?: number }>()
const query = defineModel<string>('query', { default: '' })
const desc = defineModel<boolean>('desc', { default: false })
</script>

<template>
  <SearchInput v-model="query" :placeholder="placeholder" :result-count="count" />
  <IconButton :label="desc ? 'Sorted Z to A. Sort A to Z' : 'Sorted A to Z. Sort Z to A'" @click="desc = !desc">
    <ArrowUpZA v-if="desc" /><ArrowDownAZ v-else />
  </IconButton>
</template>
