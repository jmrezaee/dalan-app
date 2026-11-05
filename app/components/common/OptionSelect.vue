<script setup lang="ts">
// A select with a "none" first entry, for optional references (a key, an identity).
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'

defineProps<{ id?: string, options: { value: string, label: string }[], noneLabel: string }>()
const model = defineModel<string | null>({ default: null })
const NONE = '__none'
const value = computed({
  get: () => model.value ?? NONE,
  set: (v: string) => { model.value = v === NONE ? null : v },
})
</script>

<template>
  <Select v-model="value">
    <SelectTrigger :id="id" class="w-full"><SelectValue /></SelectTrigger>
    <SelectContent>
      <SelectItem :value="NONE">{{ noneLabel }}</SelectItem>
      <SelectItem v-for="o in options" :key="o.value" :value="o.value">{{ o.label }}</SelectItem>
    </SelectContent>
  </Select>
</template>
