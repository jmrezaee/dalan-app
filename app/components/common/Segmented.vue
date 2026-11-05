<script setup lang="ts">
// A small radio group drawn as a segmented control (S06, S07).
import { cn } from '@/lib/utils'

defineProps<{ options: { value: string, label: string }[], label: string }>()
const model = defineModel<string>({ required: true })
</script>

<template>
  <div role="radiogroup" :aria-label="label" class="flex gap-0.5 rounded-md border border-border bg-bg p-0.5">
    <button
      v-for="o in options"
      :key="o.value"
      type="button"
      role="radio"
      :aria-checked="model === o.value"
      :class="cn('grid h-7 flex-1 place-items-center rounded px-2 text-xs text-muted transition-colors hover:text-text', model === o.value && 'bg-surface-hover text-text')"
      @click="model = o.value"
    >
      {{ o.label }}
    </button>
  </div>
</template>
