<script setup lang="ts">
// S06 (import). Paste the key, or pick a file: the dialog returns only a path and Rust reads it.
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { Switch } from '@/components/ui/switch'
import { FolderOpen } from '@lucide/vue'

const open = defineModel<boolean>('open', { required: true })
const keychain = useKeychainStore()

const form = reactive({ label: '', pem: '', path: '', passphrase: '', save: true })
const askPassphrase = ref(false)
const error = ref('')
const busy = ref(false)

watch(open, (o) => {
  if (!o) return
  Object.assign(form, { label: '', pem: '', path: '', passphrase: '', save: true })
  askPassphrase.value = false
  error.value = ''
})

async function choose() {
  try {
    const p = await keychain.pickFile()
    if (p) {
      form.path = p
      form.pem = ''
      if (!form.label) form.label = p.split(/[\\/]/).pop() ?? ''
    }
  }
  catch {
    error.value = 'Couldn\'t open the file picker.'
  }
}

async function submit() {
  if (busy.value) return
  busy.value = true
  const r = await keychain.importKey({
    label: form.label,
    pem: form.pem || null,
    path: form.pem ? null : form.path || null,
    passphrase: form.passphrase || null,
    save_passphrase: form.save && !!form.passphrase,
  })
  busy.value = false
  if (r.needsPassphrase) askPassphrase.value = true
  error.value = r.needsPassphrase ? '' : r.error
  if (!r.error) open.value = false
}
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-[520px]">
      <DialogHeader>
        <DialogTitle>Import key</DialogTitle>
        <DialogDescription>Paste a private key, or choose the file. OpenSSH and PEM formats work.</DialogDescription>
      </DialogHeader>
      <form class="flex flex-col gap-3.5" @submit.prevent="submit">
        <div class="flex flex-col gap-1.5">
          <Label for="imp-label">Label</Label>
          <Input id="imp-label" v-model="form.label" placeholder="my-old-key" autofocus autocomplete="off" />
        </div>
        <div class="flex flex-col gap-1.5">
          <Label for="imp-pem">Private key</Label>
          <Textarea id="imp-pem" v-model="form.pem" rows="6" spellcheck="false" placeholder="-----BEGIN OPENSSH PRIVATE KEY-----" class="font-mono text-xs" @input="form.path = ''" />
          <div class="flex min-w-0 items-center gap-2 text-xs text-muted">
            <Button type="button" variant="secondary" size="sm" @click="choose"><FolderOpen />Choose file…</Button>
            <span v-if="form.path" class="min-w-0 truncate font-mono" :title="form.path">{{ form.path }}</span>
          </div>
        </div>
        <div v-if="askPassphrase" class="flex flex-col gap-1.5">
          <Label for="imp-pass">Passphrase</Label>
          <PasswordInput id="imp-pass" v-model="form.passphrase" />
          <p class="text-xs text-muted">This key is protected by a passphrase.</p>
          <label class="flex items-center gap-2.5 text-sm"><Switch v-model="form.save" />Save passphrase in the vault</label>
        </div>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <div class="mt-1 flex justify-end gap-2">
          <Button type="button" variant="secondary" @click="open = false">Cancel</Button>
          <Button type="submit" :loading="busy">Import key</Button>
        </div>
      </form>
    </DialogContent>
  </Dialog>
</template>
