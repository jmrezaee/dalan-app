<script setup lang="ts">
// Re-encrypts every secret under a new master password, in one step (Rust does it in one transaction).
import { toast } from 'vue-sonner'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Label } from '@/components/ui/label'
import { CommandError, useTauri } from '~/composables/useTauri'

const open = defineModel<boolean>('open', { required: true })
const api = useTauri()
const form = reactive({ old: '', next: '', confirm: '' })
const error = ref('')
const busy = ref(false)

watch(open, (o) => {
  if (!o) return
  Object.assign(form, { old: '', next: '', confirm: '' })
  error.value = ''
})

async function submit() {
  if (busy.value) return
  if (form.next.length < 8) return void (error.value = 'Use at least 8 characters.')
  if (form.next !== form.confirm) return void (error.value = 'The new passwords don\'t match.')
  busy.value = true
  try {
    await api.vaultChangePassword(form.old, form.next)
    toast('Master password changed.')
    open.value = false
  }
  catch (e) {
    error.value = e instanceof CommandError && e.code === 'wrong_password' ? 'That password didn\'t work.' : e instanceof CommandError && e.code === 'too_many_attempts' ? e.message : 'Something went wrong. Try again.'
  }
  finally {
    busy.value = false
    Object.assign(form, { old: '', next: '', confirm: '' })
  }
}
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-[400px]">
      <DialogHeader>
        <DialogTitle>Change master password</DialogTitle>
        <DialogDescription>There's no way to recover it, so keep the new one somewhere safe.</DialogDescription>
      </DialogHeader>
      <form class="flex flex-col gap-3.5" @submit.prevent="submit">
        <div class="flex flex-col gap-1.5"><Label for="cp-old">Current password</Label><PasswordInput id="cp-old" v-model="form.old" /></div>
        <div class="flex flex-col gap-1.5"><Label for="cp-new">New password</Label><PasswordInput id="cp-new" v-model="form.next" /></div>
        <div class="flex flex-col gap-1.5"><Label for="cp-confirm">Confirm new password</Label><PasswordInput id="cp-confirm" v-model="form.confirm" /></div>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <div class="mt-1 flex justify-end gap-2">
          <Button type="button" variant="secondary" @click="open = false">Cancel</Button>
          <Button type="submit" :loading="busy">Change password</Button>
        </div>
      </form>
    </DialogContent>
  </Dialog>
</template>
