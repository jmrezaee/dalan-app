<script setup lang="ts">
// S29: a set's members.
import type { RedisBytes } from '~/types/bindings'

const props = defineProps<{ members: RedisBytes[] }>()
const rows = computed(() => props.members.map(m => [m]))
</script>

<template>
  <ValueRows label="Members" :columns="[{ label: 'Member' }]" :rows="rows" />
</template>
