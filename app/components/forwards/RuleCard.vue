<script setup lang="ts">
// S09. Status pill always has text. Error copy comes from brand.md.
import { Play, Square } from '@lucide/vue'
import { cn } from '@/lib/utils'

type State = 'stopped' | 'starting' | 'active' | 'reconnecting' | 'error'
const props = defineProps<{ kind: 'L' | 'R' | 'D'; label: string; route: string; state: State; connections?: number; error?: string }>()
defineEmits<{ toggle: [], edit: [] }>()
const pill = computed(() => ({
  stopped: { tone: 'neutral', text: 'Stopped' },
  starting: { tone: 'neutral', text: 'Starting…' },
  active: { tone: 'success', text: `Active · ${props.connections ?? 0}` },
  reconnecting: { tone: 'warning', text: 'Reconnecting' },
  error: { tone: 'danger', text: 'Error' },
} as const)[props.state])
</script>

<template>
  <div :class="cn('flex min-h-14 cursor-pointer items-center gap-3 rounded-lg border border-border bg-surface px-3 py-2 hover:border-muted/40', state === 'stopped' && 'text-muted')" @click="$emit('edit')">
    <span class="grid size-7 shrink-0 place-items-center rounded-md bg-surface-hover font-mono text-xs font-semibold">{{ kind }}</span>
    <div class="min-w-0 flex-1">
      <div class="truncate text-base text-text">{{ label }}</div>
      <div class="truncate font-mono text-xs text-muted">{{ route }}</div>
      <div v-if="state === 'error' && error" class="text-xs text-danger" role="alert">{{ error }}</div>
    </div>
    <StatusPill :tone="pill.tone">{{ pill.text }}</StatusPill>
    <IconButton :label="state === 'stopped' || state === 'error' ? 'Start tunnel' : 'Stop tunnel'" @click.stop="$emit('toggle')">
      <Play v-if="state === 'stopped' || state === 'error'" /><Square v-else />
    </IconButton>
  </div>
</template>
