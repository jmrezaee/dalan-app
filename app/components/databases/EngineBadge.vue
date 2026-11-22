<script setup lang="ts">
// The engine's two-letter badge. The label is always there for screen readers; the colour is never the only signal.
import { cn } from '@/lib/utils'
import type { Engine } from '~/types/bindings'
import { ENGINES } from '~/utils/engines'

const props = defineProps<{ engine: Engine, large?: boolean }>()
const info = computed(() => ENGINES[props.engine])
</script>

<template>
  <span :class="cn('inline-grid shrink-0 place-items-center rounded border border-current font-mono font-semibold leading-none', info.color, large ? 'h-[30px] w-9 rounded-[7px] text-[13px]' : 'h-[18px] w-[22px] text-[10px]')" role="img" :aria-label="info.label">{{ info.short }}</span>
</template>
