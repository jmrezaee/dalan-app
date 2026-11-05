<script setup lang="ts">
// Chips inside an input. Enter or comma adds, Backspace on an empty field removes the last chip.
// Suggestions come from the browser's own <datalist>.
const model = defineModel<string[]>({ default: () => [] })
defineProps<{ id?: string; suggestions?: string[] }>()
const draft = ref('')
const listId = useId()

function add() {
  const t = draft.value.trim().replace(/,$/, '').trim()
  draft.value = ''
  if (t && !model.value.some(x => x.toLowerCase() === t.toLowerCase())) model.value = [...model.value, t]
}
function onKey(e: KeyboardEvent) {
  if (e.key === 'Enter' || e.key === ',') {
    e.preventDefault() // Enter must not submit the host form
    add()
  }
  else if (e.key === 'Backspace' && !draft.value) model.value = model.value.slice(0, -1)
}
</script>

<template>
  <div class="flex min-h-8 flex-wrap items-center gap-1.5 rounded-md border border-border-input bg-bg px-1.5 py-1 focus-within:border-accent focus-within:ring-2 focus-within:ring-accent/40">
    <TagChip v-for="t in model" :key="t" :label="t" removable @remove="model = model.filter(x => x !== t)" />
    <input
      :id="id"
      v-model="draft"
      :list="listId"
      placeholder="Add tag"
      aria-label="Add tag"
      class="min-w-20 flex-1 bg-transparent text-sm outline-none placeholder:text-muted"
      @keydown="onKey"
      @blur="add"
    >
    <datalist :id="listId"><option v-for="s in suggestions?.filter(x => !model.includes(x))" :key="s" :value="s" /></datalist>
  </div>
</template>
