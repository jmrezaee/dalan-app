<script setup lang="ts">
// S10: Type -> Where -> Review. Dots plus labels, never colour alone.
import { Check } from '@lucide/vue'
import { cn } from '@/lib/utils'

defineProps<{ steps: string[]; current: number }>()
</script>

<template>
  <ol class="flex items-center gap-3" aria-label="Steps">
    <li v-for="(s, i) in steps" :key="s" class="flex items-center gap-2 text-sm" :aria-current="i === current ? 'step' : undefined">
      <span :class="cn('grid size-5 place-items-center rounded-full border text-2xs', i < current ? 'border-accent bg-accent text-on-accent' : i === current ? 'border-accent text-accent' : 'border-border-input text-muted')">
        <Check v-if="i < current" class="size-3" /><template v-else>{{ i + 1 }}</template>
      </span>
      <span :class="i === current ? 'text-text' : 'text-muted'">{{ s }}</span>
      <span v-if="i < steps.length - 1" class="h-px w-6 bg-border" aria-hidden="true" />
    </li>
  </ol>
</template>
