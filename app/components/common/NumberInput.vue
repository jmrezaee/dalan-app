<script setup lang="ts">
// Ports: 1-65535, mono digits, arrow keys step (native for type=number).
import { Input } from '@/components/ui/input'

defineProps<{ id?: string; invalid?: boolean; min?: number; max?: number; disabled?: boolean }>()
const model = defineModel<number | string>({ default: '' })
</script>

<template>
  <Input
    :id="id"
    v-model="model"
    type="number"
    inputmode="numeric"
    :min="min ?? 1"
    :max="max ?? 65535"
    :aria-invalid="invalid || undefined"
    :disabled="disabled"
    class="font-mono tabular-nums"
  />
</template>
