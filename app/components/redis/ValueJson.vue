<script setup lang="ts">
// S29: JSON, for keys of the JSON module and for strings that parse. "Tree" is the value pretty-printed and coloured;
// "Text" is exactly what is stored.
import { jsonTokens } from '~/utils/redisBytes'

const props = defineProps<{ text: string, value: unknown }>()
const mode = defineModel<string>('mode', { default: 'tree' })
const tokens = computed(() => jsonTokens(props.value))
const COLOUR = { key: 'text-ansi-blue', string: 'text-ansi-green', number: 'text-ansi-yellow', word: 'text-ansi-magenta', plain: '' } as const
</script>

<template>
  <pre v-if="mode === 'tree'" class="min-h-0 flex-1 overflow-auto p-3 font-mono text-xs leading-5" tabindex="0" aria-label="Value as JSON"><span v-for="(t, i) in tokens" :key="i" :class="COLOUR[t.kind]">{{ t.text }}</span></pre>
  <pre v-else class="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-all p-3 font-mono text-xs leading-5" tabindex="0" aria-label="Value as text">{{ text }}</pre>
</template>
