<script setup lang="ts">
// Spec: StatusDot. 8px circle. Never colour-only: aria-label + tooltip title are required by the caller's label.
import { cn } from '@/lib/utils'

type Status = 'unread' | 'waiting' | 'error' | 'idle'
const props = defineProps<{ status: Status; label: string; class?: string }>()
const colour: Record<Status, string> = {
  unread: 'bg-success',
  waiting: 'bg-warning animate-pulse motion-reduce:animate-none',
  error: 'bg-danger-solid',
  idle: 'bg-transparent border border-muted',
}
</script>

<template>
  <span
    role="img"
    :aria-label="label"
    :title="label"
    :class="cn('inline-block size-2 shrink-0 rounded-full', colour[props.status], props.class)"
  />
</template>
