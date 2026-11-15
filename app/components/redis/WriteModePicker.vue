<script setup lang="ts">
// The three write modes as radios (S27 and the tab's mode dialog). `locked` modes can't be picked here.
import type { RedisWriteMode } from '~/types/bindings'
import { cn } from '@/lib/utils'

defineProps<{ labelledby: string, locked?: RedisWriteMode[] }>()
const model = defineModel<RedisWriteMode>({ required: true })
const MODES = [
  { value: 'read_only', title: 'Read-only', text: 'Look, never change. Write commands are refused before they are sent.' },
  { value: 'confirm', title: 'Confirm writes', text: 'Every change shows what it will do and asks first.' },
  { value: 'open', title: 'Open', text: 'Edits go straight through. Dangerous commands still ask.' },
] as const
</script>

<template>
  <div role="radiogroup" :aria-labelledby="labelledby" class="flex flex-col gap-1.5">
    <button
      v-for="m in MODES"
      :key="m.value"
      type="button"
      role="radio"
      :aria-checked="model === m.value"
      :disabled="locked?.includes(m.value)"
      :class="cn('flex items-start gap-2.5 rounded-lg border border-border p-2.5 text-left disabled:opacity-50', model === m.value && 'border-accent bg-accent/5')"
      @click="model = m.value"
    >
      <span :class="cn('mt-[3px] size-3.5 shrink-0 rounded-full border-[1.5px] border-border-input', model === m.value && 'border-4 border-accent')" aria-hidden="true" />
      <span class="text-xs text-muted"><b class="text-sm font-medium text-text">{{ m.title }}</b><br>{{ m.text }}</span>
    </button>
  </div>
</template>
