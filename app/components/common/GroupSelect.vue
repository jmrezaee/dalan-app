<script setup lang="ts">
// A group picker for the editors. "No group" is the top level.
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'

const props = defineProps<{ id?: string; options: { id: string, label: string, depth: number }[]; noneLabel?: string }>()
const model = defineModel<string | null>({ default: null })
const NONE = '__none'
const value = computed({
  get: () => model.value ?? NONE,
  set: (v: string) => { model.value = v === NONE ? null : v },
})
</script>

<template>
  <Select v-model="value">
    <SelectTrigger :id="props.id" class="w-full"><SelectValue /></SelectTrigger>
    <SelectContent>
      <SelectItem :value="NONE">{{ noneLabel ?? 'No group' }}</SelectItem>
      <SelectItem v-for="o in options" :key="o.id" :value="o.id"><span :style="{ paddingLeft: `${o.depth * 12}px` }">{{ o.label }}</span></SelectItem>
    </SelectContent>
  </Select>
</template>
