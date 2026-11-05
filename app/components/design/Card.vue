<script setup lang="ts">
defineProps<{ title: string }>()
</script>

<template>
  <section class="flex flex-col gap-3.5 rounded-lg border border-border bg-surface p-5">
    <h2 class="text-lg">{{ title }}</h2>
    <slot />
  </section>
</template>
