<script setup lang="ts">
// S10: one rule. Local reaches the server's network from here; Remote shares something from here with the
// server; SOCKS turns the server into a proxy. The preview line spells out the route.
import { Sheet, SheetContent, SheetDescription, SheetTitle } from '@/components/ui/sheet'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import type { ForwardKind, ForwardSummary } from '~/types/bindings'

const open = defineModel<boolean>('open', { required: true })
const props = defineProps<{ rule: ForwardSummary | null }>()
const emit = defineEmits<{ delete: [rule: ForwardSummary] }>()
const forwards = useForwardsStore()
const hosts = useHostsStore()

const form = reactive({ kind: 'local' as string, label: '', host_id: null as string | null, bind_address: '127.0.0.1', bind_port: '' as number | string, dest_address: '127.0.0.1', dest_port: '' as number | string, autostart: false })
const error = ref('')
const busy = ref(false)
const current = computed(() => JSON.stringify(form))
const snapshot = ref('')
const dirty = computed(() => current.value !== snapshot.value)

watch(open, (o) => {
  if (!o) return
  const r = props.rule
  Object.assign(form, r
    ? { kind: r.kind, label: r.label, host_id: r.host_id, bind_address: r.bind_address, bind_port: r.bind_port, dest_address: r.dest_address ?? '127.0.0.1', dest_port: r.dest_port ?? '', autostart: r.autostart }
    : { kind: 'local', label: '', host_id: null, bind_address: '127.0.0.1', bind_port: '', dest_address: '127.0.0.1', dest_port: '', autostart: false })
  error.value = ''
  snapshot.value = current.value
})

const HINTS: Record<string, string> = {
  local: 'Reach a port on the server\'s network from this computer.',
  remote: 'Let the server reach a port on this computer.',
  dynamic: 'Use the server as a SOCKS5 proxy. Point a browser or tool at the port below.',
}
const hostOptions = computed(() => hosts.hosts.map(h => ({ value: h.id, label: `${h.label}  ·  ${h.address}` })))
const hostLabel = computed(() => hosts.hosts.find(h => h.id === form.host_id)?.label ?? 'the server')
const exposed = computed(() => !['127.0.0.1', 'localhost', '::1'].includes(form.bind_address.trim()))
const locked = computed(() => !!props.rule && props.rule.state !== 'stopped' && props.rule.state !== 'error')

const route = computed(() => {
  const port = form.bind_port || '…'
  const dest = `${form.dest_address || '…'}:${form.dest_port || '…'}`
  if (form.kind === 'remote') return `${hostLabel.value}:${port} → this computer → ${dest}`
  if (form.kind === 'dynamic') return `SOCKS on ${form.bind_address}:${port} → ${hostLabel.value} → anywhere`
  return `${form.bind_address}:${port} → ${hostLabel.value} → ${dest}`
})

async function submit() {
  if (busy.value || !form.host_id) return void (error.value = 'Choose a server for this rule.')
  busy.value = true
  error.value = await forwards.save({
    id: props.rule?.id ?? null,
    host_id: form.host_id,
    label: form.label,
    kind: form.kind as ForwardKind,
    bind_address: form.bind_address,
    bind_port: Number(form.bind_port) || 0,
    dest_address: form.kind === 'dynamic' ? null : form.dest_address,
    dest_port: form.kind === 'dynamic' ? null : Number(form.dest_port) || 0,
    autostart: form.autostart,
  })
  busy.value = false
  if (!error.value) open.value = false
}
</script>

<template>
  <Sheet v-model:open="open">
    <SheetContent class="w-[440px] max-w-full gap-0 p-0 sm:max-w-[440px]" @keydown.meta.s.prevent="submit" @keydown.ctrl.s.prevent="submit">
      <SheetDescription class="sr-only">A port forwarding rule.</SheetDescription>
      <div class="flex h-14 shrink-0 items-center gap-2 border-b border-border px-5">
        <SheetTitle class="text-base">{{ rule ? 'Edit rule' : 'New rule' }}</SheetTitle>
        <span v-if="dirty" class="text-xs text-warning">· Unsaved changes</span>
      </div>
      <form class="flex flex-1 flex-col gap-4 overflow-auto p-5" @submit.prevent="submit">
        <p v-if="locked" class="rounded-md border border-border bg-surface px-3 py-2 text-xs text-muted">Stop this rule to change it.</p>
        <fieldset :disabled="locked" class="flex flex-col gap-4">
          <div class="flex flex-col gap-1.5">
            <Label>Type</Label>
            <Segmented v-model="form.kind" label="Type" :options="[{ value: 'local', label: 'Local' }, { value: 'remote', label: 'Remote' }, { value: 'dynamic', label: 'SOCKS' }]" />
            <p class="text-xs text-muted">{{ HINTS[form.kind] }}</p>
          </div>
          <div class="flex flex-col gap-1.5"><Label for="fw-label">Label</Label><Input id="fw-label" v-model="form.label" placeholder="prod-postgres" autofocus autocomplete="off" /></div>
          <div class="flex flex-col gap-1.5"><Label for="fw-host">Through server</Label><OptionSelect id="fw-host" v-model="form.host_id" :options="hostOptions" none-label="Choose a server" /></div>
          <div class="grid grid-cols-[2fr_1fr] gap-2.5">
            <div class="flex flex-col gap-1.5"><Label for="fw-bind">{{ form.kind === 'remote' ? 'On the server' : 'On this computer' }}</Label><Input id="fw-bind" v-model="form.bind_address" class="font-mono" autocomplete="off" spellcheck="false" /></div>
            <div class="flex flex-col gap-1.5"><Label for="fw-bport">Port</Label><NumberInput id="fw-bport" v-model="form.bind_port" /></div>
          </div>
          <p v-if="exposed" class="-mt-2 text-xs text-warning">Anyone who can reach this address can use the tunnel. Use 127.0.0.1 to keep it to this computer.</p>
          <div v-if="form.kind !== 'dynamic'" class="grid grid-cols-[2fr_1fr] gap-2.5">
            <div class="flex flex-col gap-1.5"><Label for="fw-dest">{{ form.kind === 'remote' ? 'To (on this computer)' : 'To (as the server sees it)' }}</Label><Input id="fw-dest" v-model="form.dest_address" class="font-mono" autocomplete="off" spellcheck="false" /></div>
            <div class="flex flex-col gap-1.5"><Label for="fw-dport">Port</Label><NumberInput id="fw-dport" v-model="form.dest_port" /></div>
          </div>
          <p class="rounded-md border border-border bg-surface px-3 py-2 font-mono text-xs text-muted">{{ route }}</p>
          <label class="flex items-center gap-2.5 text-sm"><Switch v-model="form.autostart" />Start when Dalan unlocks</label>
        </fieldset>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <button type="submit" class="hidden" tabindex="-1" aria-hidden="true" />
      </form>
      <div class="flex h-14 shrink-0 items-center justify-end gap-2 border-t border-border px-5">
        <Button v-if="rule" variant="ghost" class="mr-auto text-danger" @click="emit('delete', rule)">Delete</Button>
        <Button variant="secondary" @click="open = false">Cancel</Button>
        <Button :loading="busy" :disabled="locked" @click="submit">Save</Button>
      </div>
    </SheetContent>
  </Sheet>
</template>
