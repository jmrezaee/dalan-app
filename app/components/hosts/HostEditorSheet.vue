<script setup lang="ts">
// S03 (minimal): label, address, port, username, password. Groups, tags, jump hosts and the rest come later.
import { Sheet, SheetContent, SheetDescription, SheetTitle } from '@/components/ui/sheet'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import type { HostSummary } from '~/types/bindings'

const hosts = useHostsStore()

const open = computed({
  get: () => hosts.editing !== null,
  set: (v) => { if (!v) hosts.editing = null },
})
const host = computed(() => (hosts.editing && hosts.editing !== 'new' ? hosts.editing : null))

const blank = () => ({ label: '', address: '', port: '' as number | string, username: '', password: '' })
const form = reactive(blank())
const clearPassword = ref(false)
const error = ref('')
const busy = ref(false)
// "Unsaved changes" = the form differs from how it looked when the sheet opened.
const current = computed(() => JSON.stringify([form, clearPassword.value]))
const snapshot = ref('')
const dirty = computed(() => current.value !== snapshot.value)

// Reset the form each time the sheet opens, from the host being edited (or blank).
watch(() => hosts.editing, (e) => {
  const h = e && e !== 'new' ? (e as HostSummary) : null
  Object.assign(form, h ? { label: h.label, address: h.address, port: h.port ?? '', username: h.username ?? '', password: '' } : blank())
  clearPassword.value = false
  error.value = ''
  snapshot.value = current.value
})

const hasSavedPassword = computed(() => !!host.value?.has_password && !clearPassword.value)

async function submit() {
  if (busy.value) return
  busy.value = true
  error.value = await hosts.save({
    id: host.value?.id ?? null,
    label: form.label,
    address: form.address,
    port: form.port === '' ? null : Number(form.port),
    username: form.username,
    password: form.password || null,
    clear_password: clearPassword.value,
  })
  busy.value = false
  if (!error.value) hosts.editing = null
}
</script>

<template>
  <Sheet v-model:open="open">
    <SheetContent
      class="w-[440px] max-w-full gap-0 p-0 sm:max-w-[440px]"
      @keydown.meta.s.prevent="submit"
      @keydown.ctrl.s.prevent="submit"
    >
      <SheetDescription class="sr-only">Connection details for this server.</SheetDescription>
      <div class="flex h-14 shrink-0 items-center gap-2 border-b border-border px-5">
        <SheetTitle class="text-base">{{ host ? 'Edit host' : 'New host' }}</SheetTitle>
        <span v-if="dirty" class="text-xs text-warning">· Unsaved changes</span>
      </div>

      <form class="flex flex-1 flex-col gap-6 overflow-auto p-5" @submit.prevent="submit">
        <section class="flex flex-col gap-3">
          <h3 class="text-2xs font-medium uppercase tracking-wider text-muted">General</h3>
          <div class="flex flex-col gap-1.5">
            <Label for="host-label">Label</Label>
            <Input id="host-label" v-model="form.label" placeholder="web-1" autofocus autocomplete="off" />
          </div>
          <div class="grid grid-cols-[3fr_1fr] gap-2.5">
            <div class="flex flex-col gap-1.5">
              <Label for="host-address">Address</Label>
              <Input id="host-address" v-model="form.address" placeholder="10.0.0.5" autocomplete="off" spellcheck="false" class="font-mono" />
            </div>
            <div class="flex flex-col gap-1.5">
              <Label for="host-port">Port</Label>
              <NumberInput id="host-port" v-model="form.port" />
            </div>
          </div>
        </section>

        <section class="flex flex-col gap-3">
          <h3 class="text-2xs font-medium uppercase tracking-wider text-muted">Credentials</h3>
          <div class="flex flex-col gap-1.5">
            <Label for="host-user">Username</Label>
            <Input id="host-user" v-model="form.username" placeholder="deploy" autocomplete="off" spellcheck="false" />
            <p class="text-xs text-muted">Leave it blank to be asked when you connect.</p>
          </div>
          <div class="flex flex-col gap-1.5">
            <Label for="host-password">Password</Label>
            <PasswordInput id="host-password" v-model="form.password" :saved="hasSavedPassword" />
            <button
              v-if="hasSavedPassword"
              type="button"
              class="self-start text-xs text-muted underline-offset-2 hover:text-text hover:underline"
              @click="clearPassword = true"
            >
              Remove saved password
            </button>
            <p v-else-if="clearPassword" class="text-xs text-muted">The saved password is removed when you save.</p>
          </div>
        </section>

        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <button type="submit" class="hidden" tabindex="-1" aria-hidden="true" />
      </form>

      <div class="flex h-14 shrink-0 items-center justify-end gap-2 border-t border-border px-5">
        <span class="mr-auto text-xs text-muted"><Kbd>⌘S</Kbd> to save</span>
        <Button variant="secondary" @click="open = false">Cancel</Button>
        <Button :loading="busy" @click="submit">Save</Button>
      </div>
    </SheetContent>
  </Sheet>
</template>
