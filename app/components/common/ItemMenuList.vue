<script setup lang="ts">
// One list of actions, drawn as a right-click menu (`context`) or a "…" menu (`dropdown`), so both always match.
import type { Component } from 'vue'
import { ContextMenuItem, ContextMenuSeparator, ContextMenuSub, ContextMenuSubContent, ContextMenuSubTrigger } from '@/components/ui/context-menu'
import { DropdownMenuItem, DropdownMenuSeparator, DropdownMenuSub, DropdownMenuSubContent, DropdownMenuSubTrigger } from '@/components/ui/dropdown-menu'

export type MenuItem = {
  label: string
  icon?: Component
  danger?: boolean
  /** Shown to the right, already formatted for this platform. */
  hint?: string
  disabled?: boolean
  onSelect?: () => void
  /** A submenu. */
  children?: MenuItem[]
  /** A line above this item. */
  separator?: boolean
}

const props = defineProps<{ items: MenuItem[], mode: 'context' | 'dropdown' }>()
const ctx = computed(() => props.mode === 'context')
const Item = computed(() => (ctx.value ? ContextMenuItem : DropdownMenuItem))
const Sep = computed(() => (ctx.value ? ContextMenuSeparator : DropdownMenuSeparator))
const Sub = computed(() => (ctx.value ? ContextMenuSub : DropdownMenuSub))
const SubTrigger = computed(() => (ctx.value ? ContextMenuSubTrigger : DropdownMenuSubTrigger))
const SubContent = computed(() => (ctx.value ? ContextMenuSubContent : DropdownMenuSubContent))
</script>

<template>
  <template v-for="(it, i) in items" :key="`${i}-${it.label}`">
    <component :is="Sep" v-if="it.separator" />
    <component :is="Sub" v-if="it.children">
      <component :is="SubTrigger" :disabled="it.disabled"><component :is="it.icon" v-if="it.icon" />{{ it.label }}</component>
      <component :is="SubContent" class="max-h-72 overflow-auto">
        <ItemMenuList :items="it.children" :mode="mode" />
      </component>
    </component>
    <component :is="Item" v-else :class="it.danger && 'text-danger'" :disabled="it.disabled" @select="it.onSelect?.()">
      <component :is="it.icon" v-if="it.icon" />{{ it.label }}<span v-if="it.hint" class="ml-auto pl-4 text-xs text-muted">{{ it.hint }}</span>
    </component>
  </template>
</template>
