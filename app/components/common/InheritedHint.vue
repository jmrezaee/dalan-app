<script setup lang="ts">
// The "from Production" badge that sits inside an empty field whose value is inherited.
defineProps<{ group: string }>()
</script>

<template>
  <span class="pointer-events-none absolute right-1.5 top-1.5 rounded bg-surface-hover px-1.5 py-0.5 text-2xs text-muted">from {{ group }}</span>
</template>
