<script setup lang="ts">
// Line settings for a serial host (S03). The device path is the host's address field.
import type { SerialConfig } from '~/types/bindings'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Label } from '@/components/ui/label'

const model = defineModel<SerialConfig>({ required: true })
const BAUDS = [1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600]
const set = (patch: Partial<SerialConfig>) => { model.value = { ...model.value, ...patch } }
</script>

<template>
  <div class="grid grid-cols-2 gap-2.5">
    <div class="flex flex-col gap-1.5">
      <Label for="serial-baud">Baud rate</Label>
      <Select :model-value="String(model.baud)" @update:model-value="(v) => set({ baud: Number(v) })">
        <SelectTrigger id="serial-baud" class="w-full"><SelectValue /></SelectTrigger>
        <SelectContent><SelectItem v-for="b in BAUDS" :key="b" :value="String(b)">{{ b }}</SelectItem></SelectContent>
      </Select>
    </div>
    <div class="flex flex-col gap-1.5">
      <Label for="serial-bits">Data bits</Label>
      <Select :model-value="String(model.data_bits)" @update:model-value="(v) => set({ data_bits: Number(v) })">
        <SelectTrigger id="serial-bits" class="w-full"><SelectValue /></SelectTrigger>
        <SelectContent><SelectItem v-for="b in [5, 6, 7, 8]" :key="b" :value="String(b)">{{ b }}</SelectItem></SelectContent>
      </Select>
    </div>
    <div class="flex flex-col gap-1.5">
      <Label for="serial-stop">Stop bits</Label>
      <Select :model-value="String(model.stop_bits)" @update:model-value="(v) => set({ stop_bits: Number(v) })">
        <SelectTrigger id="serial-stop" class="w-full"><SelectValue /></SelectTrigger>
        <SelectContent><SelectItem value="1">1</SelectItem><SelectItem value="2">2</SelectItem></SelectContent>
      </Select>
    </div>
    <div class="flex flex-col gap-1.5">
      <Label for="serial-parity">Parity</Label>
      <Select :model-value="model.parity" @update:model-value="(v) => set({ parity: String(v) })">
        <SelectTrigger id="serial-parity" class="w-full"><SelectValue /></SelectTrigger>
        <SelectContent><SelectItem value="none">None</SelectItem><SelectItem value="odd">Odd</SelectItem><SelectItem value="even">Even</SelectItem></SelectContent>
      </Select>
    </div>
    <div class="col-span-2 flex flex-col gap-1.5">
      <Label for="serial-flow">Flow control</Label>
      <Select :model-value="model.flow" @update:model-value="(v) => set({ flow: String(v) })">
        <SelectTrigger id="serial-flow" class="w-full"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem value="none">None</SelectItem>
          <SelectItem value="hardware">Hardware (RTS/CTS)</SelectItem>
          <SelectItem value="software">Software (XON/XOFF)</SelectItem>
        </SelectContent>
      </Select>
    </div>
  </div>
</template>
