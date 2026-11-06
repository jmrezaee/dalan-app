<script setup lang="ts">
// S03/S04: the jump hosts a connection goes through. Inherit (from the group), connect directly, or an ordered list.
import { ArrowDown, ArrowUp, X } from '@lucide/vue'

const props = defineProps<{
  /** Hosts that can be picked (the editor leaves out the host being edited). */
  options: { id: string, label: string, address: string }[]
  /** The chain this item would inherit, and from which group. */
  inheritedIds: string[]
  inheritedFrom?: string | null
  /** Shown at the end of the preview line. */
  target: string
}>()
const mode = defineModel<string>('mode', { required: true })
const ids = defineModel<string[]>('ids', { required: true })

const adding = ref<string | null>(null)
const labelOf = (id: string) => props.options.find(o => o.id === id)?.label ?? 'a deleted host'
const available = computed(() => props.options.filter(o => !ids.value.includes(o.id)).map(o => ({ value: o.id, label: `${o.label}  ·  ${o.address}` })))
watch(adding, (v) => {
  if (v) {
    ids.value = [...ids.value, v]
    nextTick(() => { adding.value = null })
  }
})
function move(i: number, by: number) {
  const next = [...ids.value]
  const [x] = next.splice(i, 1)
  next.splice(i + by, 0, x!)
  ids.value = next
}
const shown = computed(() => (mode.value === 'custom' ? ids.value : mode.value === 'none' ? [] : props.inheritedIds))
const route = computed(() => ['you', ...shown.value.map(labelOf), props.target || 'this host'].join(' → '))
</script>

<template>
  <div class="flex flex-col gap-2">
    <Segmented v-model="mode" label="Jump hosts" :options="[{ value: 'inherit', label: inheritedFrom ? `From ${inheritedFrom}` : 'Inherit' }, { value: 'none', label: 'Direct' }, { value: 'custom', label: 'Custom' }]" />
    <div v-if="mode === 'custom'" class="rounded-md border border-border bg-surface">
      <div v-for="(id, i) in ids" :key="id" class="flex h-9 items-center gap-1 border-b border-border px-2 text-sm">
        <span class="mr-1 w-4 text-xs text-muted">{{ i + 1 }}</span>
        <span class="truncate">{{ labelOf(id) }}</span>
        <span class="ml-auto flex">
          <IconButton :label="`Move ${labelOf(id)} up`" size="xs" :disabled="i === 0" @click="move(i, -1)"><ArrowUp /></IconButton>
          <IconButton :label="`Move ${labelOf(id)} down`" size="xs" :disabled="i === ids.length - 1" @click="move(i, 1)"><ArrowDown /></IconButton>
          <IconButton :label="`Remove ${labelOf(id)} from the chain`" size="xs" @click="ids = ids.filter(x => x !== id)"><X /></IconButton>
        </span>
      </div>
      <div class="p-1.5"><OptionSelect v-model="adding" :options="available" none-label="Add jump host…" /></div>
    </div>
    <ol class="flex flex-col gap-0 text-sm" :aria-label="route">
      <li v-for="(name, i) in ['You', ...shown.map(labelOf), target || 'This host']" :key="i" class="flex items-center gap-2.5">
        <span class="flex w-3 flex-col items-center self-stretch" aria-hidden="true">
          <span :class="['w-px flex-1', i === 0 ? 'bg-transparent' : 'bg-border-input']" />
          <span :class="['size-2.5 rounded-full border', i === 0 || i === shown.length + 1 ? 'border-accent bg-accent' : 'border-border-input bg-surface']" />
          <span :class="['w-px flex-1', i === shown.length + 1 ? 'bg-transparent' : 'bg-border-input']" />
        </span>
        <span class="truncate py-1.5" :class="i === 0 || i === shown.length + 1 ? 'font-medium' : 'text-muted'">{{ name }}</span>
      </li>
    </ol>
  </div>
</template>
