<script setup lang="ts">
// Forward this computer's SSH agent to the server (S03, S04). Off unless chosen.
import type { AgentConfig } from '~/types/bindings'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'

defineProps<{ info: AgentConfig | null }>()
const model = defineModel<string>({ required: true })
const kinds = [
  { v: 'inherit', t: 'Inherit' },
  { v: 'on', t: 'Forward agent' },
  { v: 'off', t: 'Don\'t forward' },
]
</script>

<template>
  <div class="flex flex-col gap-2">
    <Select v-model="model">
      <SelectTrigger class="w-full" aria-label="Agent forwarding"><SelectValue /></SelectTrigger>
      <SelectContent>
        <SelectItem v-for="k in kinds" :key="k.v" :value="k.v">{{ k.t }}</SelectItem>
      </SelectContent>
    </Select>
    <p v-if="model === 'inherit' && info?.inherited_from" class="text-xs text-muted">{{ info.inherited ? 'Forwards' : 'Doesn\'t forward' }} the agent, set in {{ info.inherited_from }}.</p>
    <p class="text-xs text-muted">Lets the server use your SSH agent while you are connected. Anyone with admin rights on that server can use it too, so turn it on only for servers you trust. Keys saved in Dalan are never forwarded.</p>
    <p v-if="info && !info.available" class="text-xs text-warning">No SSH agent is running on this computer, so nothing would be forwarded.</p>
  </div>
</template>
