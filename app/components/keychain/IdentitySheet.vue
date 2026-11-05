<script setup lang="ts">
// S07: a username plus how to sign in, shared by many hosts and groups.
import { Sheet, SheetContent, SheetDescription, SheetTitle } from '@/components/ui/sheet'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import type { IdentitySummary } from '~/types/bindings'

const open = defineModel<boolean>('open', { required: true })
const props = defineProps<{ identity: IdentitySummary | null }>()
const keychain = useKeychainStore()

const form = reactive({ label: '', username: '', how: 'password', password: '', key_id: null as string | null })
const clearPassword = ref(false)
const error = ref('')
const busy = ref(false)

watch(open, (o) => {
  if (!o) return
  const i = props.identity
  Object.assign(form, { label: i?.label ?? '', username: i?.username ?? '', how: i?.key_id ? 'key' : 'password', password: '', key_id: i?.key_id ?? null })
  clearPassword.value = false
  error.value = ''
})

const keyOptions = computed(() => keychain.keys.map(k => ({ value: k.id, label: `${k.label}  ·  ${k.key_type}` })))
const hasOwnPassword = computed(() => !!props.identity?.has_password && !clearPassword.value)

async function submit() {
  if (busy.value) return
  busy.value = true
  const usesKey = form.how === 'key'
  const r = await keychain.saveIdentity({
    id: props.identity?.id ?? null,
    label: form.label,
    username: form.username,
    password: usesKey ? null : form.password || null,
    clear_password: usesKey || clearPassword.value,
    key_id: usesKey ? form.key_id : null,
  })
  busy.value = false
  error.value = r.error
  if (!r.error) open.value = false
}
</script>

<template>
  <Sheet v-model:open="open">
    <SheetContent class="w-[440px] max-w-full gap-0 p-0 sm:max-w-[440px]">
      <SheetDescription class="sr-only">A username and how to sign in, shared by many hosts.</SheetDescription>
      <div class="flex h-14 shrink-0 items-center border-b border-border px-5"><SheetTitle class="text-base">{{ identity ? 'Edit identity' : 'New identity' }}</SheetTitle></div>
      <form class="flex flex-1 flex-col gap-4 overflow-auto p-5" @submit.prevent="submit">
        <p class="text-sm text-muted">A username and how to sign in, shared by many hosts.</p>
        <div class="flex flex-col gap-1.5"><Label for="id-label">Label</Label><Input id="id-label" v-model="form.label" placeholder="deploy (production)" autofocus autocomplete="off" /></div>
        <div class="flex flex-col gap-1.5"><Label for="id-user">Username</Label><Input id="id-user" v-model="form.username" placeholder="deploy" autocomplete="off" spellcheck="false" /></div>
        <div class="flex flex-col gap-1.5">
          <Label>Sign in with</Label>
          <Segmented v-model="form.how" label="Sign in with" :options="[{ value: 'password', label: 'Password' }, { value: 'key', label: 'Key' }]" />
        </div>
        <div v-if="form.how === 'password'" class="flex flex-col gap-1.5">
          <Label for="id-pass">Password</Label>
          <PasswordInput id="id-pass" v-model="form.password" :saved="hasOwnPassword" />
          <button v-if="hasOwnPassword" type="button" class="self-start text-xs text-muted underline-offset-2 hover:text-text hover:underline" @click="clearPassword = true">Remove saved password</button>
        </div>
        <div v-else class="flex flex-col gap-1.5">
          <Label for="id-key">Key</Label>
          <OptionSelect id="id-key" v-model="form.key_id" :options="keyOptions" none-label="Choose a key" />
          <p v-if="!keyOptions.length" class="text-xs text-muted">No keys yet. Generate or import one first.</p>
        </div>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <button type="submit" class="hidden" tabindex="-1" aria-hidden="true" />
      </form>
      <div class="flex h-14 shrink-0 items-center justify-end gap-2 border-t border-border px-5">
        <Button variant="secondary" @click="open = false">Cancel</Button>
        <Button :loading="busy" @click="submit">Save identity</Button>
      </div>
    </SheetContent>
  </Sheet>
</template>
