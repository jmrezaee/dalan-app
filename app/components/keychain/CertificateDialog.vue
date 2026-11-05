<script setup lang="ts">
// Attach an OpenSSH user certificate (id_ed25519-cert.pub) to a key: key and certificate become one item.
// Paste the text, or choose the file; Rust reads the file and checks the certificate.
import { FolderOpen } from '@lucide/vue'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Textarea } from '@/components/ui/textarea'
import { Label } from '@/components/ui/label'
import type { KeySummary } from '~/types/bindings'

const open = defineModel<boolean>('open', { required: true })
const props = defineProps<{ keyItem: KeySummary | null }>()
const keychain = useKeychainStore()

const text = ref('')
const path = ref('')
const error = ref('')
const busy = ref(false)

watch(open, (o) => {
  if (!o) return
  text.value = ''
  path.value = ''
  error.value = ''
})

async function choose() {
  try {
    const p = await useTauri().pickFile()
    if (p) {
      path.value = p
      text.value = ''
    }
  }
  catch {
    error.value = 'Couldn\'t open the file picker.'
  }
}

async function submit() {
  if (busy.value || !props.keyItem) return
  if (!text.value.trim() && !path.value) {
    error.value = 'Paste the certificate or choose its file.'
    return
  }
  busy.value = true
  const r = await keychain.setCertificate(props.keyItem.id, text.value || null, text.value ? null : path.value || null)
  busy.value = false
  error.value = r.error
  if (!r.error) open.value = false
}
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-[520px]">
      <DialogHeader>
        <DialogTitle>Certificate for {{ keyItem?.label }}</DialogTitle>
        <DialogDescription>A user certificate signed by your organisation's CA. Dalan signs in with the key and this certificate together.</DialogDescription>
      </DialogHeader>
      <form class="flex flex-col gap-3.5" @submit.prevent="submit">
        <div class="flex flex-col gap-1.5">
          <Label for="cert-text">Certificate</Label>
          <Textarea id="cert-text" v-model="text" rows="5" spellcheck="false" placeholder="ssh-ed25519-cert-v01@openssh.com AAAA…" class="font-mono text-xs" @input="path = ''" />
          <div class="flex min-w-0 items-center gap-2 text-xs text-muted">
            <Button type="button" variant="secondary" size="sm" @click="choose"><FolderOpen />Choose file…</Button>
            <span v-if="path" class="min-w-0 truncate font-mono" :title="path">{{ path }}</span>
          </div>
        </div>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <div class="mt-1 flex justify-end gap-2">
          <Button type="button" variant="secondary" @click="open = false">Cancel</Button>
          <Button type="submit" :loading="busy">Attach certificate</Button>
        </div>
      </form>
    </DialogContent>
  </Dialog>
</template>
