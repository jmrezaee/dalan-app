<script setup lang="ts">
// S02/S16. Spec: HostCard. Selection and focus are conveyed by ring + text, not colour alone.
import { Ellipsis, LoaderCircle } from '@lucide/vue'
import { cn } from '@/lib/utils'

defineProps<{
  label: string
  user: string
  address: string
  port?: number
  osLetter?: string
  tags?: string[]
  selected?: boolean
  connecting?: boolean
  connected?: boolean
  dragging?: boolean
  multiSelect?: boolean
  checked?: boolean
}>()
defineEmits<{ open: []; menu: [] }>()
</script>

<template>
  <div
    role="row"
    tabindex="0"
    :aria-selected="selected"
    :class="cn(
      'group flex gap-3 rounded-lg border border-border bg-surface p-3 transition-colors duration-100 hover:border-muted/40 hover:bg-surface-raised',
      selected && 'ring-2 ring-accent',
      dragging && 'opacity-50',
    )"
    @dblclick="$emit('open')"
    @keydown.enter="$emit('open')"
  >
    <div class="relative grid size-8 shrink-0 place-items-center rounded-md bg-surface-hover text-2xs font-semibold">
      <LoaderCircle v-if="connecting" class="size-4 animate-spin text-accent" aria-label="Connecting" />
      <template v-else>{{ osLetter ?? 'S' }}</template>
      <span v-if="connected" class="absolute -right-0.5 -top-0.5 size-2 rounded-full bg-success ring-2 ring-surface" title="Connected" />
    </div>
    <div class="min-w-0 flex-1">
      <div class="flex items-center gap-2">
        <input v-if="multiSelect" type="checkbox" :checked="checked" :aria-label="`Select ${label}`" class="accent-[var(--accent)]">
        <span class="truncate text-base">{{ label }}</span>
      </div>
      <div class="truncate text-xs text-muted">{{ user }}@<span class="font-mono">{{ address }}</span><template v-if="port && port !== 22">:{{ port }}</template></div>
      <div v-if="tags?.length" class="mt-1.5 flex flex-wrap gap-1"><TagChip v-for="t in tags" :key="t" :label="t" /></div>
    </div>
    <IconButton label="Host actions" size="xs" class="opacity-0 group-hover:opacity-100 focus-visible:opacity-100" @click.stop="$emit('menu')"><Ellipsis /></IconButton>
  </div>
</template>
