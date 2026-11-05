<script setup lang="ts">
// "New ▾": primary action plus an attached chevron that opens a menu.
import { ChevronDown } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'

defineProps<{ label: string }>()
defineEmits<{ click: [] }>()
const open = defineModel<boolean>('open', { default: false })
</script>

<template>
  <div class="inline-flex">
    <Button class="rounded-r-none" @click="$emit('click')">
      <slot name="icon" />{{ label }}
    </Button>
    <DropdownMenu v-model:open="open">
      <DropdownMenuTrigger as-child>
        <Button size="icon" aria-label="More create options" aria-haspopup="menu" class="w-7 rounded-l-none border-l border-on-accent/30">
          <ChevronDown class="size-3.5 transition-transform" :class="open && 'rotate-180'" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end"><slot /></DropdownMenuContent>
    </DropdownMenu>
  </div>
</template>
