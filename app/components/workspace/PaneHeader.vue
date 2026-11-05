<script setup lang="ts">
import { Maximize2, X } from '@lucide/vue'
import { cn } from '@/lib/utils'

defineProps<{ title: string; status: 'unread' | 'waiting' | 'error' | 'idle'; statusLabel: string; focused?: boolean; broadcasting?: boolean }>()
defineEmits<{ maximize: []; close: [] }>()
</script>

<template>
  <div :class="cn('flex h-7 items-center gap-2 border border-border bg-surface px-2 text-xs rounded-md', focused && 'border-accent', broadcasting && 'border-t-2 border-t-warning')">
    <StatusDot :status="status" :label="statusLabel" />
    <span class="truncate">{{ title }}</span>
    <StatusPill v-if="broadcasting" tone="warning">Broadcasting</StatusPill>
    <span class="ml-auto flex items-center gap-0.5">
      <IconButton label="Maximize pane" size="xs" @click="$emit('maximize')"><Maximize2 /></IconButton>
      <IconButton label="Close pane" size="xs" @click="$emit('close')"><X /></IconButton>
    </span>
  </div>
</template>
