<script setup lang="ts">
import type { Component } from 'vue'

defineProps<{ icon: Component; title: string; text: string }>()
</script>

<template>
  <div class="mx-auto flex max-w-[360px] flex-col items-center gap-2 py-8 text-center">
    <component :is="icon" class="size-8 text-muted" :stroke-width="1.75" aria-hidden="true" />
    <h2 class="text-lg">{{ title }}</h2>
    <p class="text-muted">{{ text }}</p>
    <div class="mt-2 flex items-center gap-2"><slot /></div>
  </div>
</template>
