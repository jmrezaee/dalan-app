<script setup lang="ts">
// Rename a key, and remember or forget its passphrase.
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import type { KeySummary } from '~/types/bindings'

const open = defineModel<boolean>('open', { required: true })
const props = defineProps<{ keyItem: KeySummary | null }>()
const keychain = useKeychainStore()

const form = reactive({ label: '', passphrase: '' })
const forget = ref(false)
const error = ref('')
const busy = ref(false)

watch(open, (o) => {
  if (!o) return
  Object.assign(form, { label: props.keyItem?.label ?? '', passphrase: '' })
  forget.value = false
  error.value = ''
})

async function submit() {
  if (busy.value || !props.keyItem) return
  busy.value = true
  const r = await keychain.updateKey({ id: props.keyItem.id, label: form.label, save_passphrase: form.passphrase || null, forget_passphrase: forget.value })
  busy.value = false
  error.value = r.error
  if (!r.error) open.value = false
}
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-[400px]">
      <DialogHeader>
        <DialogTitle>Edit key</DialogTitle>
        <DialogDescription>The key itself can't be changed. Delete it and import another if you need that.</DialogDescription>
      </DialogHeader>
      <form class="flex flex-col gap-3.5" @submit.prevent="submit">
        <div class="flex flex-col gap-1.5">
          <Label for="edit-label">Label</Label>
          <Input id="edit-label" v-model="form.label" autofocus autocomplete="off" />
        </div>
        <div v-if="keyItem?.encrypted" class="flex flex-col gap-1.5">
          <Label for="edit-pass">{{ keyItem.passphrase_saved ? 'Replace saved passphrase' : 'Save passphrase' }}</Label>
          <PasswordInput id="edit-pass" v-model="form.passphrase" />
          <button v-if="keyItem.passphrase_saved && !forget" type="button" class="self-start text-xs text-muted underline-offset-2 hover:text-text hover:underline" @click="forget = true">Forget the saved passphrase</button>
          <p v-else-if="forget" class="text-xs text-muted">Dalan will ask for the passphrase on every connect.</p>
        </div>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <div class="mt-1 flex justify-end gap-2">
          <Button type="button" variant="secondary" @click="open = false">Cancel</Button>
          <Button type="submit" :loading="busy">Save</Button>
        </div>
      </form>
    </DialogContent>
  </Dialog>
</template>
