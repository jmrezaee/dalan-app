<script setup lang="ts">
// "Show details" under a connection card: what Dalan did, step by step, with a Copy button. Plain text, no secrets.
import { toast } from 'vue-sonner'

const props = defineProps<{ steps: string[], open?: boolean }>()
const shown = ref(props.open ?? false)
watch(() => props.open, (o) => { if (o) shown.value = true })

async function copy() {
  try {
    await navigator.clipboard.writeText(props.steps.join('\n'))
    toast('Details copied.')
  }
  catch {
    toast.error('Couldn\'t copy the details.')
  }
}
</script>

<template>
  <div v-if="steps.length" class="flex flex-col gap-2">
    <div class="flex items-center justify-between">
      <button type="button" class="text-xs text-muted underline-offset-2 hover:text-text hover:underline" :aria-expanded="shown" @click="shown = !shown">
        {{ shown ? 'Hide details' : 'Show details' }}
      </button>
      <button v-if="shown" type="button" class="text-xs text-muted underline-offset-2 hover:text-text hover:underline" @click="copy">Copy details</button>
    </div>
    <ol v-if="shown" aria-label="Connection details" class="max-h-40 overflow-auto rounded-md bg-bg px-2.5 py-2 font-mono text-xs leading-5 text-muted">
      <li v-for="(s, i) in steps" :key="i">{{ s }}</li>
    </ol>
  </div>
</template>
