<script setup lang="ts">
// Terminal theme picker for hosts and groups. "Inherit" = no value of its own (S03, S04).
import { terminalThemeLabels } from '~/utils/terminal-themes'

defineProps<{ id?: string, inheritedTheme?: string | null, inheritedFrom?: string | null }>()
const model = defineModel<string | null>({ default: null })
const options = Object.entries(terminalThemeLabels).map(([value, label]) => ({ value, label }))
</script>

<template>
  <div class="flex flex-col gap-1.5">
    <OptionSelect :id="id" v-model="model" :options="options" none-label="Inherit" />
    <p v-if="!model && inheritedTheme" class="text-xs text-muted">Uses {{ terminalThemeLabels[inheritedTheme] ?? inheritedTheme }} from {{ inheritedFrom }}.</p>
  </div>
</template>
