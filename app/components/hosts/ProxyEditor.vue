<script setup lang="ts">
// Proxy for the connection to this host (S03, S04): inherit, none, SOCKS5 or HTTP.
import type { ProxyChoice } from '~/types/bindings'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'

defineProps<{ inherited: string | null, inheritedFrom: string | null }>()
const model = defineModel<ProxyChoice>({ required: true })
const kinds = [
  { v: 'inherit', t: 'Inherit' },
  { v: 'none', t: 'No proxy' },
  { v: 'socks5', t: 'SOCKS5' },
  { v: 'http', t: 'HTTP (CONNECT)' },
]
const port = computed({
  get: () => model.value.port ?? '',
  set: (v: string | number) => { model.value = { ...model.value, port: v === '' ? null : Number(v) } },
})
</script>

<template>
  <div class="flex flex-col gap-2.5">
    <Select :model-value="model.kind" @update:model-value="(v) => (model = { ...model, kind: String(v) })">
      <SelectTrigger class="w-full" aria-label="Proxy type"><SelectValue /></SelectTrigger>
      <SelectContent>
        <SelectItem v-for="k in kinds" :key="k.v" :value="k.v">{{ k.t }}</SelectItem>
      </SelectContent>
    </Select>
    <p v-if="model.kind === 'inherit' && inherited" class="text-xs text-muted">Uses {{ inherited }} from {{ inheritedFrom }}.</p>
    <div v-if="model.kind === 'socks5' || model.kind === 'http'" class="flex gap-2">
      <div class="flex flex-1 flex-col gap-1.5">
        <Label for="proxy-host">Proxy address</Label>
        <Input id="proxy-host" v-model="model.host" placeholder="proxy.example.com" autocomplete="off" spellcheck="false" />
      </div>
      <div class="flex w-24 flex-col gap-1.5">
        <Label for="proxy-port">Port</Label>
        <Input id="proxy-port" v-model="port" type="number" min="1" max="65535" placeholder="1080" />
      </div>
    </div>
    <p v-if="model.kind === 'socks5' || model.kind === 'http'" class="text-xs text-muted">Only proxies that need no sign-in. With jump hosts, the proxy carries the connection to the first one.</p>
  </div>
</template>
