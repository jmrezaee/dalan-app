<script setup lang="ts">
// S29: a string. Text when it is text, JSON when it parses, hex always; a binary value opens as hex and is read-only.
// Over 1 MB only the first 64 KB are loaded until "Load all".
import type { RedisBytes } from '~/types/bindings'
import { byteLength, bytesOf, count, hexDump, parsedJson, sizeText } from '~/utils/redisBytes'

const props = defineProps<{ value: RedisBytes, totalBytes: number, truncated: boolean, loading: boolean, writable: boolean, saveLabel: string }>()
defineEmits<{ loadAll: [], save: [text: string] }>()
// A preview (over 1 MB) and a binary value are read-only: saving either would lose bytes.
const editable = computed(() => props.writable && props.value.kind === 'text' && !props.truncated)

const json = computed(() => (props.truncated ? undefined : parsedJson(props.value)))
const binary = computed(() => props.value.kind === 'binary')
const mode = ref('text')
// Each value opens in the view that suits it: hex for binary, JSON when it parses, text otherwise.
watch(() => props.value, () => { mode.value = binary.value ? 'hex' : json.value !== undefined ? 'json' : 'text' }, { immediate: true })
const options = computed(() => [
  ...(binary.value ? [] : [{ value: 'text', label: 'Text' }]),
  ...(json.value !== undefined ? [{ value: 'json', label: 'JSON' }] : []),
  { value: 'hex', label: 'Hex' },
])
const hex = computed(() => hexDump(bytesOf(props.value)))
const loaded = computed(() => byteLength(props.value))
</script>

<template>
  <div class="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
    <Segmented v-model="mode" label="Show as" :options="options" class="w-auto" />
    <span v-if="binary" class="text-xs text-muted">Binary values open as hex and are read-only.</span>
  </div>
  <ValueJson v-if="mode === 'json' && json !== undefined" :text="value.kind === 'text' ? value.text : ''" :value="json" mode="tree" />
  <pre v-else-if="mode === 'hex'" class="min-h-0 flex-1 overflow-auto p-3 font-mono text-xs leading-5" tabindex="0" aria-label="Value as hex">{{ hex }}</pre>
  <ValueEditor v-else-if="editable && value.kind === 'text'" :text="value.text" :save-label="saveLabel" @save="(t: string) => $emit('save', t)" />
  <pre v-else class="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-all p-3 font-mono text-xs leading-5" tabindex="0" aria-label="Value as text">{{ value.kind === 'text' ? value.text : '' }}</pre>
  <div class="flex min-h-9 shrink-0 items-center gap-2 border-t border-border px-3 text-xs text-muted">
    <template v-if="truncated">
      <span>Showing the first {{ sizeText(loaded) }} of {{ sizeText(totalBytes) }}.</span>
      <Button variant="ghost" size="sm" class="text-accent" :loading="loading" @click="$emit('loadAll')">Load all</Button>
    </template>
    <span v-else>{{ count(totalBytes) }} {{ totalBytes === 1 ? 'byte' : 'bytes' }} · {{ binary ? 'binary' : 'text' }}</span>
    <span v-if="mode === 'hex' && loaded > 64 * 1024" class="ml-auto">The hex view shows the first 64 KB.</span>
  </div>
</template>
