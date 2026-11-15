<script setup lang="ts">
// S29: the text of a string or a JSON-module key, editable. It ends in "Review change" (Confirm mode) or "Save"
// (Open mode); nothing is sent until then. A JSON key's text must parse.
import { jsonProblem } from '~/utils/redisBytes'

const props = defineProps<{ text: string, saveLabel: string, json?: boolean }>()
const emit = defineEmits<{ save: [text: string] }>()
const draft = ref(props.text)
// The value was read again (a save went through, or Reload): start from what is stored.
watch(() => props.text, (t) => { draft.value = t })
const dirty = computed(() => draft.value !== props.text)
const problem = computed(() => (props.json && dirty.value ? jsonProblem(draft.value) : ''))
</script>

<template>
  <textarea v-model="draft" class="min-h-0 flex-1 resize-none bg-bg p-3 font-mono text-xs leading-5 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" aria-label="Value as text" spellcheck="false" autocapitalize="off" autocomplete="off" />
  <div class="flex min-h-10 shrink-0 items-center gap-2 border-t border-border px-3 text-xs">
    <span v-if="problem" role="alert" class="text-danger">{{ problem }}</span>
    <span v-else-if="dirty" class="text-muted">Not saved yet.</span>
    <span class="ml-auto flex gap-1.5">
      <Button v-if="dirty" variant="ghost" size="sm" @click="draft = text">Discard</Button>
      <Button size="sm" :disabled="!dirty || !!problem" @click="emit('save', draft)">{{ saveLabel }}</Button>
    </span>
  </div>
</template>
