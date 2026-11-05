<script setup lang="ts">
// S08: install a key's public half on a server, then link the key to that host. Host-key and
// password questions come up inside this dialog, the same ones a terminal tab would ask.
import { CircleAlert, Check, LoaderCircle } from '@lucide/vue'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import type { KeySummary, PromptKind, SessionEvent } from '~/types/bindings'
import { useTauri } from '~/composables/useTauri'

const open = defineModel<boolean>('open', { required: true })
const props = defineProps<{ keyItem: KeySummary | null }>()
const hosts = useHostsStore()
const api = useTauri()

const form = reactive({ hostId: null as string | null, dir: '~/.ssh', file: 'authorized_keys' })
const phase = ref<'idle' | 'running' | 'done' | 'error'>('idle')
const step = ref('')
const message = ref('')
const prompt = ref<{ id: string, kind: PromptKind } | null>(null)
let sessionId = ''

watch(open, (o) => {
  if (!o) return
  Object.assign(form, { hostId: null, dir: '~/.ssh', file: 'authorized_keys' })
  phase.value = 'idle'
  prompt.value = null
  message.value = ''
})

const hostOptions = computed(() => hosts.hosts.map(h => ({ value: h.id, label: `${h.label}  ·  ${h.address}` })))
const hostLabel = computed(() => hosts.hosts.find(h => h.id === form.hostId)?.label ?? 'the server')

function onEvent(e: SessionEvent) {
  if (e.type === 'state') {
    if (e.state === 'connecting') step.value = `Connecting to ${hostLabel.value}…`
    else if (e.state === 'authenticating') step.value = 'Signing in…'
    else if (e.state === 'closed' && phase.value === 'running') phase.value = 'idle' // cancelled
  }
  else if (e.type === 'prompt') {
    prompt.value = { id: e.prompt_id, kind: e.kind }
  }
  else if (e.type === 'error') {
    phase.value = 'error'
    prompt.value = null
    message.value = e.message
  }
  else if (e.type === 'exit') {
    phase.value = 'done'
    hosts.refresh()
  }
}

async function start() {
  if (!props.keyItem || !form.hostId) return
  phase.value = 'running'
  step.value = `Connecting to ${hostLabel.value}…`
  message.value = ''
  try {
    sessionId = (await api.keyExportToHost(props.keyItem.id, form.hostId, form.dir, form.file, onEvent)).session_id
  }
  catch {
    phase.value = 'error'
    message.value = 'Something went wrong. Try again.'
  }
}

async function reply(accept: boolean, text: string | null, remember: boolean) {
  const p = prompt.value
  if (!p) return
  prompt.value = null
  await api.sessionRespond(sessionId, p.id, { accept, text, remember }).catch(() => {})
}
function close() {
  if (phase.value === 'running') api.sessionClose(sessionId)
  open.value = false
}
</script>

<template>
  <Dialog :open="open" @update:open="(v: boolean) => { if (!v) close() }">
    <DialogContent class="sm:max-w-[420px]">
      <DialogHeader>
        <DialogTitle>Install key on a server</DialogTitle>
        <DialogDescription>
          Adds <b class="font-semibold text-text">{{ keyItem?.label }}</b>'s public key to the server's authorized_keys, then links the key to that host.
        </DialogDescription>
      </DialogHeader>

      <form class="flex flex-col gap-3.5" @submit.prevent="start">
        <div class="flex flex-col gap-1.5">
          <Label for="exp-host">Server</Label>
          <OptionSelect id="exp-host" v-model="form.hostId" :options="hostOptions" none-label="Choose a server" />
        </div>
        <div class="grid grid-cols-2 gap-2.5">
          <div class="flex flex-col gap-1.5"><Label for="exp-dir">Folder</Label><Input id="exp-dir" v-model="form.dir" class="font-mono" autocomplete="off" spellcheck="false" /></div>
          <div class="flex flex-col gap-1.5"><Label for="exp-file">File</Label><Input id="exp-file" v-model="form.file" class="font-mono" autocomplete="off" spellcheck="false" /></div>
        </div>

        <div v-if="phase !== 'idle'" class="flex flex-col gap-2 rounded-md border border-border bg-surface p-3 text-xs" aria-live="polite">
          <PromptPanel v-if="prompt" :kind="prompt.kind" :host="hostLabel" @reply="reply" />
          <span v-else-if="phase === 'running'" class="flex items-center gap-2"><LoaderCircle class="size-3.5 animate-spin text-accent" />{{ step }}</span>
          <template v-else-if="phase === 'done'">
            <span class="flex items-center gap-2 text-success"><Check class="size-3.5" />Key added (folder 700, file 600)</span>
            <span class="flex items-center gap-2 text-success"><Check class="size-3.5" />Linked to {{ hostLabel }}</span>
          </template>
          <span v-else-if="phase === 'error'" class="flex items-start gap-2 text-danger"><CircleAlert class="mt-0.5 size-3.5 shrink-0" />{{ message }}</span>
        </div>

        <div class="mt-1 flex justify-end gap-2">
          <Button type="button" variant="secondary" @click="close">{{ phase === 'done' ? 'Done' : 'Cancel' }}</Button>
          <Button v-if="phase !== 'done'" type="submit" :loading="phase === 'running'" :disabled="!form.hostId">Install key</Button>
        </div>
      </form>
    </DialogContent>
  </Dialog>
</template>
