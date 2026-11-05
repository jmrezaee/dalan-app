<script setup lang="ts">
import { Folder } from '@lucide/vue'
import { cn } from '@/lib/utils'

defineProps<{ label: string; count: number; dropTarget?: boolean }>()
</script>

<template>
  <div
    tabindex="0"
    :class="cn('flex items-center gap-3 rounded-lg border border-border bg-surface p-3 hover:border-muted/40 hover:bg-surface-raised', dropTarget && 'border-accent bg-accent/10')"
  >
    <Folder class="size-5 text-muted" aria-hidden="true" />
    <div class="min-w-0">
      <div class="truncate text-base">{{ label }}</div>
      <div class="text-xs text-muted">{{ count }} {{ count === 1 ? 'host' : 'hosts' }}</div>
    </div>
  </div>
</template>
