<script setup lang="ts">
// S22 transfer row.
import { ArrowDown, ArrowUp, X } from '@lucide/vue'
import { cn } from '@/lib/utils'

defineProps<{ name: string; direction: 'up' | 'down'; percent: number; speed?: string; eta?: string; status?: 'running' | 'done' | 'failed' }>()
defineEmits<{ cancel: [] }>()
</script>

<template>
  <div class="flex items-center gap-2.5">
    <component :is="direction === 'up' ? ArrowUp : ArrowDown" class="size-4 text-muted" aria-hidden="true" />
    <div class="flex-1">
      <div class="flex justify-between"><span class="truncate">{{ name }}</span><span class="text-xs text-muted">{{ status === 'failed' ? 'Failed' : status === 'done' ? 'Done' : [speed, eta].filter(Boolean).join(' · ') }}</span></div>
      <div class="mt-1.5 h-1 rounded-sm bg-surface-hover" role="progressbar" :aria-valuenow="percent" aria-valuemin="0" aria-valuemax="100" :aria-label="`${name} progress`">
        <div :class="cn('h-1 rounded-sm', status === 'failed' ? 'bg-danger-solid' : status === 'done' ? 'bg-success' : 'bg-accent')" :style="{ width: `${percent}%` }" />
      </div>
    </div>
    <IconButton label="Cancel transfer" size="xs" @click="$emit('cancel')"><X /></IconButton>
  </div>
</template>
