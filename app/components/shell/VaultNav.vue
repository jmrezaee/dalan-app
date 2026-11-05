<script setup lang="ts">
import type { Component } from 'vue'
import { cn } from '@/lib/utils'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'

defineProps<{
  items: { id: string; label: string; icon: Component; count?: number }[]
  active: string
  collapsed?: boolean
}>()
defineEmits<{ select: [id: string] }>()
</script>

<template>
  <nav aria-label="Vault" :class="cn('flex flex-col gap-0.5 border-r border-border bg-surface p-2', collapsed ? 'w-14' : 'w-[220px]')">
    <Tooltip v-for="it in items" :key="it.id" :disabled="!collapsed">
      <TooltipTrigger as-child>
        <a
          href="#"
          :aria-current="it.id === active ? 'page' : undefined"
          :aria-label="collapsed ? it.label : undefined"
          :class="cn(
            'relative flex h-8 items-center gap-2 rounded-md px-2.5 text-sm',
            it.id === active ? 'bg-surface-hover text-text before:absolute before:inset-y-1 before:left-0 before:w-0.5 before:rounded-full before:bg-accent' : 'text-muted hover:bg-surface-hover hover:text-text',
            collapsed && 'justify-center px-0',
          )"
          @click.prevent="$emit('select', it.id)"
        >
          <component :is="it.icon" class="size-4 shrink-0" aria-hidden="true" />
          <template v-if="!collapsed">
            <span class="flex-1 truncate">{{ it.label }}</span>
            <Badge v-if="it.count !== undefined">{{ it.count }}</Badge>
          </template>
        </a>
      </TooltipTrigger>
      <TooltipContent side="right">{{ it.label }}</TooltipContent>
    </Tooltip>
  </nav>
</template>
