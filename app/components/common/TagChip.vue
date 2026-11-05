<script setup lang="ts">
import { X } from '@lucide/vue'

defineProps<{ label: string; removable?: boolean }>()
defineEmits<{ remove: [] }>()
</script>

<template>
  <span class="inline-flex h-5 items-center gap-1 rounded-full bg-surface-hover px-2 text-xs text-text">
    {{ label }}
    <button v-if="removable" type="button" :aria-label="`Remove tag ${label}`" class="grid size-3.5 place-items-center rounded-full text-muted hover:text-text" @click="$emit('remove')">
      <X class="size-3" />
    </button>
  </span>
</template>
