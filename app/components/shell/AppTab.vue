<script setup lang="ts">
// One tab in the TabBar (S01). The close button is a separate focusable button.
import { X } from '@lucide/vue'
import { cn } from '@/lib/utils'

defineProps<{
  label: string
  active?: boolean
  dragging?: boolean
  dropTarget?: boolean
  unsaved?: boolean
  closable?: boolean
}>()
defineEmits<{ select: []; close: [] }>()
</script>

<template>
  <div
    :class="cn(
      'group relative flex h-8 min-w-[120px] max-w-[220px] items-center gap-2 rounded-t-md px-2.5 text-sm',
      active ? 'bg-bg text-text' : 'text-muted hover:bg-surface-hover',
      dragging && 'opacity-90 shadow-drag',
      dropTarget && 'outline-2 outline-dashed outline-accent',
    )"
  >
    <button type="button" role="tab" :aria-selected="active" class="flex min-w-0 flex-1 items-center gap-2 text-left" @click="$emit('select')">
      <slot name="leading" />
      <span class="truncate">{{ label }}</span>
    </button>
    <span v-if="unsaved" class="size-1.5 shrink-0 rounded-full bg-muted" title="Unsaved changes" aria-label="Unsaved changes" />
    <button
      v-if="closable"
      type="button"
      :aria-label="`Close ${label}`"
      :class="cn('grid size-4 shrink-0 place-items-center rounded-sm hover:bg-surface-hover', active ? 'opacity-100' : 'opacity-0 group-hover:opacity-100 focus-visible:opacity-100')"
      @click="$emit('close')"
    >
      <X class="size-3" />
    </button>
    <span v-if="dropTarget" class="absolute -bottom-5 left-0 text-2xs text-accent">Drop to split</span>
  </div>
</template>
