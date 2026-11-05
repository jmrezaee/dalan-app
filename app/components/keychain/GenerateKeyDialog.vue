<script setup lang="ts">
// S06 (generate). The private key is created in Rust and never shown.
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import type { KeyKind } from '~/types/bindings'

const open = defineModel<boolean>('open', { required: true })
const keychain = useKeychainStore()

const form = reactive({ label: '', type: 'ed25519', ecdsa: 'ecdsa_p256' as string, passphrase: '', save: true })
const error = ref('')
const busy = ref(false)

const HINTS: Record<string, string> = {
  ed25519: 'Ed25519 is short, fast and works on any OpenSSH from 2014 on.',
  ecdsa: 'ECDSA works almost everywhere. Pick the curve below.',
  rsa4096: 'RSA 4096 is for old servers that don\'t support Ed25519. Generating it takes a few seconds.',
}

watch(open, (o) => {
  if (!o) return
  Object.assign(form, { label: '', type: 'ed25519', ecdsa: 'ecdsa_p256', passphrase: '', save: true })
  error.value = ''
})

async function submit() {
  if (busy.value) return
  busy.value = true
  const kind = (form.type === 'ecdsa' ? form.ecdsa : form.type === 'rsa4096' ? 'rsa4096' : 'ed25519') as KeyKind
  const r = await keychain.generate({ label: form.label, kind, passphrase: form.passphrase || null, save_passphrase: form.save && !!form.passphrase })
  busy.value = false
  error.value = r.error
  if (!r.error) open.value = false
}
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-[400px]">
      <DialogHeader>
        <DialogTitle>Generate key</DialogTitle>
        <DialogDescription>The private key never leaves this computer.</DialogDescription>
      </DialogHeader>
      <form class="flex flex-col gap-3.5" @submit.prevent="submit">
        <div class="flex flex-col gap-1.5">
          <Label for="gen-label">Label</Label>
          <Input id="gen-label" v-model="form.label" placeholder="deploy-ed25519" autofocus autocomplete="off" />
        </div>
        <div class="flex flex-col gap-1.5">
          <Label>Type</Label>
          <Segmented v-model="form.type" label="Key type" :options="[{ value: 'ed25519', label: 'Ed25519' }, { value: 'ecdsa', label: 'ECDSA' }, { value: 'rsa4096', label: 'RSA 4096' }]" />
          <Segmented v-if="form.type === 'ecdsa'" v-model="form.ecdsa" label="ECDSA curve" :options="[{ value: 'ecdsa_p256', label: 'P-256' }, { value: 'ecdsa_p384', label: 'P-384' }, { value: 'ecdsa_p521', label: 'P-521' }]" />
          <p class="text-xs text-muted">{{ HINTS[form.type] }}</p>
        </div>
        <div class="flex flex-col gap-1.5">
          <Label for="gen-pass">Passphrase (optional)</Label>
          <PasswordInput id="gen-pass" v-model="form.passphrase" />
        </div>
        <label v-if="form.passphrase" class="flex items-center gap-2.5 text-sm"><Switch v-model="form.save" />Save passphrase in the vault</label>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <div class="mt-1 flex justify-end gap-2">
          <Button type="button" variant="secondary" @click="open = false">Cancel</Button>
          <Button type="submit" :loading="busy">Generate and save</Button>
        </div>
      </form>
    </DialogContent>
  </Dialog>
</template>
