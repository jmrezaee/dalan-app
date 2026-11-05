<script setup lang="ts">
defineProps<{ title: string }>()
</script>

<template>
  <header class="flex h-12 items-center justify-between gap-3 border-b border-border px-6">
    <slot name="title"><h1 class="text-xl">{{ title }}</h1></slot>
    <div class="flex items-center gap-2"><slot /></div>
  </header>
</template>
