<script setup lang="ts">
// S33 "Connecting": the steps of opening a database, one row each: every SSH hop, then reaching Redis, then the sign-in.
import { Check, X } from '@lucide/vue'
import type { RedisStep } from '~/stores/redis'

defineProps<{ name: string, steps: RedisStep[] }>()
defineEmits<{ cancel: [] }>()
const stateWord = { pending: 'waiting', active: 'in progress', done: 'done', failed: 'failed' } as const
</script>

<template>
  <div class="flex w-[420px] max-w-full flex-col gap-1 rounded-lg border border-border bg-surface p-4" :data-testid="tid('redis-steps')">
    <h2 class="mb-1.5 text-base font-semibold">Opening {{ name }}</h2>
    <ol aria-live="polite" class="flex flex-col">
      <li v-for="(s, i) in steps" :key="i" class="flex min-h-7 items-center gap-2.5 py-0.5" :data-state="s.state">
        <span class="grid size-3.5 shrink-0 place-items-center">
          <Check v-if="s.state === 'done'" class="size-3.5 text-success" aria-hidden="true" />
          <X v-else-if="s.state === 'failed'" class="size-3.5 text-danger" aria-hidden="true" />
          <span v-else-if="s.state === 'active'" class="size-3.5 animate-spin rounded-full border-2 border-accent border-r-transparent" aria-hidden="true" />
          <span v-else class="size-3.5 rounded-full border-[1.5px] border-border-input" aria-hidden="true" />
        </span>
        <span :class="s.state === 'pending' ? 'text-muted' : ''">{{ s.label }}<span class="sr-only">, {{ stateWord[s.state] }}</span></span>
        <span v-if="s.detail" :class="['min-w-0 truncate text-xs', s.state === 'failed' ? 'text-danger' : 'text-muted']" :title="s.detail">{{ s.detail }}</span>
      </li>
      <li v-if="!steps.length" class="flex min-h-7 items-center gap-2.5 text-muted"><span class="size-3.5 animate-spin rounded-full border-2 border-accent border-r-transparent" aria-hidden="true" />Getting ready</li>
    </ol>
    <div class="mt-2 flex justify-end"><Button variant="secondary" size="sm" @click="$emit('cancel')">Cancel</Button></div>
  </div>
</template>
