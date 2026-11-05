<script setup lang="ts">
// S01. 40px bar, macOS leaves 78px for the traffic lights. Empty area is the drag region.
import { Plus } from '@lucide/vue'
import IconButton from '../common/IconButton.vue'
const { label: shortcutLabel } = useShortcutLabel()

defineProps<{ mac?: boolean }>()
defineEmits<{ new: [] }>()
</script>

<template>
  <div class="flex h-10 items-end border-b border-border bg-chrome px-2" :class="mac && 'pl-[78px]'" data-tauri-drag-region>
    <div role="tablist" class="flex min-w-0 items-end gap-0.5 overflow-x-auto"><slot /></div>
    <IconButton label="New tab" :shortcut="shortcutLabel('new-tab')" class="mx-1 mb-1" @click="$emit('new')"><Plus /></IconButton>
    <div class="h-full flex-1" data-tauri-drag-region />
  </div>
</template>
