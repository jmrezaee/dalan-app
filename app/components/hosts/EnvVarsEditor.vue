<script setup lang="ts">
// Environment variables for a host or group (S03, S04). The mode says how they combine with the groups above.
import { Plus, Trash2 } from '@lucide/vue'
import type { EnvVar, InheritedVar } from '~/types/bindings'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'

defineProps<{ inherited: InheritedVar[] }>()
const mode = defineModel<string>('mode', { required: true })
const vars = defineModel<EnvVar[]>('vars', { required: true })
const modes = [
  { v: 'inherit', t: 'Use inherited' },
  { v: 'merge', t: 'Add to inherited' },
  { v: 'replace', t: 'Replace inherited' },
]
const add = () => { vars.value = [...vars.value, { name: '', value: '' }] }
const remove = (i: number) => { vars.value = vars.value.filter((_, j) => j !== i) }
</script>

<template>
  <div class="flex flex-col gap-2.5">
    <RadioGroup v-model="mode" class="flex flex-wrap gap-x-4 gap-y-1.5" aria-label="How variables combine">
      <div v-for="m in modes" :key="m.v" class="flex items-center gap-2">
        <RadioGroupItem :id="`env-mode-${m.v}`" :value="m.v" />
        <Label :for="`env-mode-${m.v}`" class="font-normal">{{ m.t }}</Label>
      </div>
    </RadioGroup>
    <ul v-if="inherited.length" class="flex flex-col gap-0.5 text-xs text-muted">
      <li v-for="i in inherited" :key="i.name" class="font-mono">{{ i.name }}={{ i.value }} <span class="font-sans">from {{ i.from }}</span></li>
    </ul>
    <template v-if="mode !== 'inherit'">
      <div v-for="(v, i) in vars" :key="i" class="flex items-center gap-2">
        <Input v-model="v.name" :aria-label="`Name ${i + 1}`" placeholder="NAME" class="w-[40%] font-mono text-xs" autocomplete="off" spellcheck="false" />
        <Input v-model="v.value" :aria-label="`Value ${i + 1}`" placeholder="value" class="flex-1 font-mono text-xs" autocomplete="off" spellcheck="false" />
        <IconButton label="Remove variable" @click="remove(i)"><Trash2 /></IconButton>
      </div>
      <div><Button type="button" variant="secondary" size="sm" @click="add"><Plus />Add variable</Button></div>
      <p class="text-xs text-muted">The server only accepts names its AcceptEnv setting allows (often LANG and LC_*). Values are stored unencrypted, so don't put secrets here.</p>
    </template>
  </div>
</template>
