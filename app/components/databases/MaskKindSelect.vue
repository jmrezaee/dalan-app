<script setup lang="ts">
// The masking rules (E5) as a select, in our words.
import type { MaskKind } from '~/types/bindings'

defineProps<{ label: string }>()
const model = defineModel<MaskKind>({ required: true })
const KINDS: [MaskKind, string][] = [
  ['keep', 'Keep'],
  ['null', 'Empty (NULL)'],
  ['fixed', 'Fixed value'],
  ['fake_email', 'Fake email'],
  ['fake_name', 'Fake name'],
  ['fake_phone', 'Fake phone'],
  ['fake_address', 'Fake address'],
  ['fake_iban', 'Fake IBAN'],
  ['hash', 'Hash (same shape, joins kept)'],
  ['shuffle', 'Shuffle within the column'],
  ['truncate', 'Cut to n characters'],
  ['date_shift', 'Shift dates'],
]
</script>

<template>
  <select v-model="model" class="h-7 rounded-md border border-border-input bg-bg px-1 text-xs" :aria-label="label">
    <option v-for="k in KINDS" :key="k[0]" :value="k[0]">{{ k[1] }}</option>
  </select>
</template>
