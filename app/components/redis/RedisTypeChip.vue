<script setup lang="ts">
// The key's type as a small chip. The word is always there, so the colour is never the only signal.
import { cn } from '@/lib/utils'
import { TYPE_LABEL } from '~/utils/redisBytes'

const props = defineProps<{ type: string }>()
const COLOUR: Record<string, string> = {
  string: 'text-ansi-blue', hash: 'text-ansi-magenta', list: 'text-ansi-green', set: 'text-ansi-yellow', zset: 'text-ansi-red', stream: 'text-ansi-cyan', json: 'text-ansi-white',
}
const label = computed(() => TYPE_LABEL[props.type] ?? props.type)
</script>

<template>
  <span :class="cn('inline-block min-w-11 shrink-0 rounded border border-current px-1 text-center font-mono text-[10px] font-semibold uppercase leading-4 tracking-wide', COLOUR[type] ?? 'text-muted')">{{ label }}</span>
</template>
