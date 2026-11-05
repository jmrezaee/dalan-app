<script setup lang="ts">
// S04: a group's name, where it sits, and the defaults its hosts inherit (username, port, password).
import { Sheet, SheetContent, SheetDescription, SheetTitle } from '@/components/ui/sheet'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import type { EnvVar, ProxyChoice, GroupSummary, InheritedVar } from '~/types/bindings'

const hosts = useHostsStore()
const keychain = useKeychainStore()
const snippets = useSnippetsStore()
const snippetOptions = computed(() => snippets.snippets.map(s => ({ value: s.id, label: s.label })))
const identityOptions = computed(() => keychain.identities.map(i => ({ value: i.id, label: `${i.label}  ·  ${i.username}` })))
const keyOptions = computed(() => keychain.keys.map(k => ({ value: k.id, label: `${k.label}  ·  ${k.key_type}` })))

const open = computed({
  get: () => hosts.editingGroup !== null,
  set: (v) => { if (!v) hosts.editingGroup = null },
})
const group = computed(() => (hosts.editingGroup && hosts.editingGroup !== 'new' ? hosts.editingGroup : null))

const api = useTauri()
/** Variables from the groups above, and whether this item's own were loaded (so a failed load can't wipe them on save). */
const envInherited = ref<InheritedVar[]>([])
const proxyInherited = ref<{ inherited: string | null, inherited_from: string | null } | null>(null)
const envLoaded = ref(true)
async function loadEnv(id: string) {
  envLoaded.value = false
  try {
    const c = await api.envGet('group', id)
    form.env_mode = c.mode
    form.env_vars = c.vars.map(v => ({ ...v }))
    envInherited.value = c.inherited
    const px = await api.proxyGet('group', id)
    form.proxy = { ...px.own }
    proxyInherited.value = px
    snapshot.value = current.value
    envLoaded.value = true
  }
  catch {
    error.value = 'Couldn\'t load the variables. Close and reopen to edit them.'
  }
}

const blank = () => ({ label: '', parent_id: hosts.currentGroupId as string | null, port: '' as number | string, username: '', password: '', identity_id: null as string | null, key_id: null as string | null, startup_snippet_id: null as string | null, jump_mode: 'inherit', jump_host_ids: [] as string[], terminal_theme: null as string | null, env_mode: 'inherit', env_vars: [] as EnvVar[], proxy: { kind: 'inherit', host: '', port: null } as ProxyChoice })
const form = reactive(blank())
const clearPassword = ref(false)
const error = ref('')
const busy = ref(false)
const current = computed(() => JSON.stringify([form, clearPassword.value]))
const snapshot = ref('')
const dirty = computed(() => current.value !== snapshot.value)

watch(() => hosts.editingGroup, (e) => {
  const g = e && e !== 'new' ? (e as GroupSummary) : null
  Object.assign(form, g ? { label: g.label, parent_id: g.parent_id, port: g.port ?? '', username: g.username ?? '', password: '', identity_id: g.identity_id, key_id: g.key_id, startup_snippet_id: g.startup_snippet_id, jump_mode: g.jump_mode, jump_host_ids: [...g.jump_host_ids], terminal_theme: hosts.themes.groups[g.id]?.own ?? null, env_mode: 'inherit', env_vars: [], proxy: { kind: 'inherit', host: '', port: null } } : blank())
  clearPassword.value = false
  error.value = ''
  snapshot.value = current.value
  envInherited.value = []
  proxyInherited.value = null
  envLoaded.value = true
  if (g) loadEnv(g.id)
})

const inherited = computed(() => hosts.inheritedFor(form.parent_id))
const parentTheme = computed(() => {
  const g = form.parent_id ? hosts.themes.groups[form.parent_id] : null
  return g?.own ? { inherited: g.own, inherited_from: hosts.groupLabel(form.parent_id!) } : g ?? null
})
const hasOwnPassword = computed(() => !!group.value?.has_password && !clearPassword.value)

async function submit() {
  if (busy.value) return
  // Checked here too: the proxy is saved after the item, and a late failure would leave a half-saved copy open.
  if ((form.proxy.kind === 'socks5' || form.proxy.kind === 'http') && (!form.proxy.host.trim() || !form.proxy.port)) {
    error.value = 'Enter the proxy\'s address and port.'
    return
  }
  busy.value = true
  error.value = await hosts.saveGroup({
    id: group.value?.id ?? null,
    parent_id: form.parent_id,
    label: form.label,
    port: form.port === '' ? null : Number(form.port),
    username: form.username,
    password: form.password || null,
    clear_password: clearPassword.value,
    identity_id: form.identity_id,
    key_id: form.key_id,
    startup_snippet_id: form.startup_snippet_id,
    jump_mode: form.jump_mode,
    jump_host_ids: form.jump_host_ids,
  }, form.terminal_theme, envLoaded.value ? { mode: form.env_mode, vars: form.env_vars.filter((v: EnvVar) => v.name.trim()).map((v: EnvVar) => ({ name: v.name.trim(), value: v.value })) } : undefined, envLoaded.value ? form.proxy : undefined)
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
            <Label for="group-identity">Identity</Label>
            <OptionSelect id="group-identity" v-model="form.identity_id" :options="identityOptions" none-label="None" />
          </div>
          <div class="flex flex-col gap-1.5">
            <Label for="group-key">Key</Label>
            <OptionSelect id="group-key" v-model="form.key_id" :options="keyOptions" none-label="None" />
            <p v-if="!form.key_id && inherited.key" class="text-xs text-muted">Uses the key {{ keychain.keyLabel(inherited.key.value) ?? 'from a group' }} from {{ inherited.key.group }}.</p>
          </div>
          <div class="flex flex-col gap-1.5">
            <Label for="group-password">Password</Label>
            <PasswordInput id="group-password" v-model="form.password" :saved="hasOwnPassword" />
            <button v-if="hasOwnPassword" type="button" class="self-start text-xs text-muted underline-offset-2 hover:text-text hover:underline" @click="clearPassword = true">Remove saved password</button>
            <p v-else-if="clearPassword" class="text-xs text-muted">The saved password is removed when you save.</p>
            <p v-if="!hasOwnPassword && !form.password && inherited.password" class="text-xs text-muted">Uses the password from {{ inherited.password }}.</p>
          </div>
        </section>

        <section class="flex flex-col gap-3">
          <h3 class="text-2xs font-medium uppercase tracking-wider text-muted">Terminal</h3>
          <div class="flex flex-col gap-1.5">
            <Label for="group-theme">Theme</Label>
            <ThemeSelect id="group-theme" v-model="form.terminal_theme" :inherited-theme="parentTheme?.inherited" :inherited-from="parentTheme?.inherited_from" />
          </div>
        </section>

        <section class="flex flex-col gap-3">
          <h3 class="text-2xs font-medium uppercase tracking-wider text-muted">Environment variables</h3>
          <EnvVarsEditor v-model:mode="form.env_mode" v-model:vars="form.env_vars" :inherited="envInherited" />
        </section>

        <section class="flex flex-col gap-3">
          <h3 class="text-2xs font-medium uppercase tracking-wider text-muted">Proxy</h3>
          <ProxyEditor v-model="form.proxy" :inherited="proxyInherited?.inherited ?? null" :inherited-from="proxyInherited?.inherited_from ?? null" />
        </section>

        <section class="flex flex-col gap-3">
          <h3 class="text-2xs font-medium uppercase tracking-wider text-muted">Connection</h3>
          <div class="flex flex-col gap-1.5">
            <Label>Jump hosts</Label>
            <ChainPicker
              v-model:mode="form.jump_mode"
              v-model:ids="form.jump_host_ids"
              :options="hosts.hosts"
              :inherited-ids="inherited.jump?.host_ids ?? []"
              :inherited-from="inherited.jump?.group"
              :target="form.label || 'hosts here'"
            />
          </div>
          <div class="flex flex-col gap-1.5">
            <Label for="group-startup">Startup snippet</Label>
            <OptionSelect id="group-startup" v-model="form.startup_snippet_id" :options="snippetOptions" none-label="None" />
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
