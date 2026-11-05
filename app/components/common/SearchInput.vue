<script setup lang="ts">
import { Search, X } from '@lucide/vue'
import { Input } from '@/components/ui/input'

defineProps<{ placeholder?: string; resultCount?: number }>()
const model = defineModel<string>({ default: '' })
const input = ref<InstanceType<typeof Input> | null>(null)
// Esc clears first, then blurs.
function onEsc(e: KeyboardEvent) {
  if (model.value) model.value = ''
  else (e.target as HTMLElement).blur()
}
</script>

<template>
  <div class="relative w-64">
    <Search class="pointer-events-none absolute left-2.5 top-2 size-4 text-muted" aria-hidden="true" />
    <Input ref="input" v-model="model" :placeholder="placeholder ?? 'Search'" aria-label="Search" class="px-8" @keydown.esc.prevent="onEsc" />
    <button
      v-if="model"
      type="button"
      aria-label="Clear search"
      class="absolute right-1.5 top-1.5 grid size-5 place-items-center rounded-sm text-muted hover:text-text"
      @click="model = ''"
    >
      <X class="size-3.5" />
    </button>
    <span v-if="resultCount !== undefined && model" class="sr-only" aria-live="polite">{{ resultCount }} results</span>
  </div>
</template>
