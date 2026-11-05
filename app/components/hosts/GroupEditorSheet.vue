<script setup lang="ts">
// S04: a group's name, where it sits, and the defaults its hosts inherit (username, port, password).
import { Sheet, SheetContent, SheetDescription, SheetTitle } from '@/components/ui/sheet'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import type { GroupSummary } from '~/types/bindings'

const hosts = useHostsStore()

const open = computed({
  get: () => hosts.editingGroup !== null,
  set: (v) => { if (!v) hosts.editingGroup = null },
})
const group = computed(() => (hosts.editingGroup && hosts.editingGroup !== 'new' ? hosts.editingGroup : null))

const blank = () => ({ label: '', parent_id: hosts.currentGroupId as string | null, port: '' as number | string, username: '', password: '' })
const form = reactive(blank())
const clearPassword = ref(false)
const error = ref('')
const busy = ref(false)
const current = computed(() => JSON.stringify([form, clearPassword.value]))
const snapshot = ref('')
const dirty = computed(() => current.value !== snapshot.value)

watch(() => hosts.editingGroup, (e) => {
  const g = e && e !== 'new' ? (e as GroupSummary) : null
  Object.assign(form, g ? { label: g.label, parent_id: g.parent_id, port: g.port ?? '', username: g.username ?? '', password: '' } : blank())
  clearPassword.value = false
  error.value = ''
  snapshot.value = current.value
})

const inherited = computed(() => hosts.inheritedFor(form.parent_id))
const hasOwnPassword = computed(() => !!group.value?.has_password && !clearPassword.value)

async function submit() {
  if (busy.value) return
  busy.value = true
  error.value = await hosts.saveGroup({
    id: group.value?.id ?? null,
    parent_id: form.parent_id,
    label: form.label,
    port: form.port === '' ? null : Number(form.port),
    username: form.username,
    password: form.password || null,
    clear_password: clearPassword.value,
  })
  busy.value = false
  if (!error.value) hosts.editingGroup = null
}
</script>

<template>
  <Sheet v-model:open="open">
    <SheetContent class="w-[440px] max-w-full gap-0 p-0 sm:max-w-[440px]" @keydown.meta.s.prevent="submit" @keydown.ctrl.s.prevent="submit">
      <SheetDescription class="sr-only">A group and the defaults its hosts inherit.</SheetDescription>
      <div class="flex h-14 shrink-0 items-center gap-2 border-b border-border px-5">
        <SheetTitle class="text-base">{{ group ? 'Edit group' : 'New group' }}</SheetTitle>
        <span v-if="dirty" class="text-xs text-warning">· Unsaved changes</span>
      </div>

      <form class="flex flex-1 flex-col gap-6 overflow-auto p-5" @submit.prevent="submit">
        <section class="flex flex-col gap-3">
          <h3 class="text-2xs font-medium uppercase tracking-wider text-muted">General</h3>
          <div class="flex flex-col gap-1.5">
            <Label for="group-label">Name</Label>
            <Input id="group-label" v-model="form.label" placeholder="Production" autofocus autocomplete="off" />
          </div>
          <div class="flex flex-col gap-1.5">
            <Label for="group-parent">Inside</Label>
            <GroupSelect id="group-parent" v-model="form.parent_id" :options="hosts.groupOptions(group?.id)" none-label="Top level" />
          </div>
        </section>

        <section class="flex flex-col gap-3">
          <h3 class="text-2xs font-medium uppercase tracking-wider text-muted">Defaults for hosts in this group</h3>
          <div class="flex flex-col gap-1.5">
            <Label for="group-user">Username</Label>
            <div class="relative">
              <Input id="group-user" v-model="form.username" :placeholder="inherited.username?.value ?? 'deploy'" autocomplete="off" spellcheck="false" :class="inherited.username && !form.username && 'pr-24'" />
              <InheritedHint v-if="inherited.username && !form.username" :group="inherited.username.group" />
            </div>
          </div>
          <div class="flex flex-col gap-1.5">
            <Label for="group-port">Port</Label>
            <div class="w-28"><NumberInput id="group-port" v-model="form.port" :placeholder="String(inherited.port?.value ?? 22)" /></div>
            <p v-if="form.port === '' && inherited.port" class="text-2xs text-muted">from {{ inherited.port.group }}</p>
          </div>
          <div class="flex flex-col gap-1.5">
            <Label for="group-password">Password</Label>
            <PasswordInput id="group-password" v-model="form.password" :saved="hasOwnPassword" />
            <button v-if="hasOwnPassword" type="button" class="self-start text-xs text-muted underline-offset-2 hover:text-text hover:underline" @click="clearPassword = true">Remove saved password</button>
            <p v-else-if="clearPassword" class="text-xs text-muted">The saved password is removed when you save.</p>
            <p v-if="!hasOwnPassword && !form.password && inherited.password" class="text-xs text-muted">Uses the password from {{ inherited.password }}.</p>
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
