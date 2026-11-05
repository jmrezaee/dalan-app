<script setup lang="ts">
import { cn } from '@/lib/utils'

type Tone = 'success' | 'warning' | 'danger' | 'neutral'
const props = defineProps<{ tone: Tone }>()
const tones: Record<Tone, string> = {
  success: 'bg-success/15 text-success',
  warning: 'bg-warning/15 text-warning',
  danger: 'bg-danger/15 text-danger',
  neutral: 'bg-surface-hover text-muted',
}
</script>

<template>
  <span :class="cn('inline-flex h-5 items-center gap-1 rounded-full px-2 text-xs font-medium', tones[props.tone])"><slot /></span>
</template>
