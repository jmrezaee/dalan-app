<script setup lang="ts">
// S29: a sorted set's members, highest score first.
import type { RedisBytes } from '~/types/bindings'

const props = defineProps<{ members: { member: RedisBytes, score: string }[] }>()
const rows = computed(() => props.members.map(m => [m.score, m.member]))
</script>

<template>
  <ValueRows label="Members" :columns="[{ label: 'Score ↓', width: '120px' }, { label: 'Member' }]" :rows="rows" />
</template>
