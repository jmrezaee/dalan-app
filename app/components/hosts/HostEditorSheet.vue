<script setup lang="ts">
// S03: label, address, port, group, tags, username, password. Empty username, port and password
// inherit from the group, and the field says where from. Jump hosts, env vars and the rest come later.
import { Sheet, SheetContent, SheetDescription, SheetTitle } from '@/components/ui/sheet'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import type { AgentConfig, EnvVar, ProxyChoice, HostSummary, InheritedVar } from '~/types/bindings'

const hosts = useHostsStore()
const keychain = useKeychainStore()
const snippets = useSnippetsStore()
const snippetOptions = computed(() => snippets.snippets.map(s => ({ value: s.id, label: s.label })))
const identityOptions = computed(() => keychain.identities.map(i => ({ value: i.id, label: `${i.label}  ·  ${i.username}` })))
const keyOptions = computed(() => keychain.keys.map(k => ({ value: k.id, label: `${k.label}  ·  ${k.key_type}` })))

const open = computed({
  get: () => hosts.editing !== null,
  set: (v) => { if (!v) hosts.editing = null },
})
const host = computed(() => (hosts.editing && hosts.editing !== 'new' ? hosts.editing : null))

const api = useTauri()
/** Variables from the groups above, and whether this item's own were loaded (so a failed load can't wipe them on save). */
const envInherited = ref<InheritedVar[]>([])
const agentInfo = ref<AgentConfig | null>(null)
const proxyInherited = ref<{ inherited: string | null, inherited_from: string | null } | null>(null)
const envLoaded = ref(true)
async function loadEnv(id: string) {
  envLoaded.value = false
  try {
    const c = await api.envGet('host', id)
    form.env_mode = c.mode
    form.env_vars = c.vars.map(v => ({ ...v }))
    envInherited.value = c.inherited
    const px = await api.proxyGet('host', id)
    form.proxy = { ...px.own }
    proxyInherited.value = px
    const ag = await api.agentGet('host', id)
    form.agent = ag.own.kind
    agentInfo.value = ag
    snapshot.value = current.value
    envLoaded.value = true
  }
  catch {
    error.value = 'Couldn\'t load the variables. Close and reopen to edit them.'
  }
}

const blank = () => ({ label: '', address: '', port: '' as number | string, username: '', password: '', group_id: hosts.currentGroupId as string | null, tags: [] as string[], identity_id: null as string | null, key_id: null as string | null, startup_snippet_id: null as string | null, jump_mode: 'inherit', jump_host_ids: [] as string[], terminal_theme: null as string | null, env_mode: 'inherit', env_vars: [] as EnvVar[], proxy: { kind: 'inherit', host: '', port: null } as ProxyChoice, agent: 'inherit' })
const form = reactive(blank())
const clearPassword = ref(false)
const error = ref('')
const busy = ref(false)
// "Unsaved changes" = the form differs from how it looked when the sheet opened.
const current = computed(() => JSON.stringify([form, clearPassword.value]))
const snapshot = ref('')
const dirty = computed(() => current.value !== snapshot.value)

// Reset the form each time the sheet opens, from the host being edited (or blank, in the group you are viewing).
watch(() => hosts.editing, (e) => {
  const h = e && e !== 'new' ? (e as HostSummary) : null
  Object.assign(form, h ? { label: h.label, address: h.address, port: h.port ?? '', username: h.username ?? '', password: '', group_id: h.group_id, tags: [...h.tags], identity_id: h.identity_id, key_id: h.key_id, startup_snippet_id: h.startup_snippet_id, jump_mode: h.jump_mode, jump_host_ids: [...h.jump_host_ids], terminal_theme: hosts.themes.hosts[h.id]?.own ?? null, env_mode: 'inherit', env_vars: [], proxy: { kind: 'inherit', host: '', port: null }, agent: 'inherit' } : blank())
  clearPassword.value = false
  error.value = ''
  snapshot.value = current.value
  envInherited.value = []
  proxyInherited.value = null
  agentInfo.value = null
  envLoaded.value = true
  if (h) loadEnv(h.id)
})

const inherited = computed(() => hosts.inheritedFor(form.group_id))
/** The theme the chosen group (or its ancestors) would give this host. */
const groupTheme = computed(() => {
  const g = form.group_id ? hosts.themes.groups[form.group_id] : null
  const label = form.group_id ? hosts.groupLabel(form.group_id) : null
  return g?.own ? { inherited: g.own, inherited_from: label } : g ?? null
})
const hasOwnPassword = computed(() => !!host.value?.has_password && !clearPassword.value)

async function submit() {
  if (busy.value) return
  // Checked here too: the proxy is saved after the item, and a late failure would leave a half-saved copy open.
  if ((form.proxy.kind === 'socks5' || form.proxy.kind === 'http') && (!form.proxy.host.trim() || !form.proxy.port)) {
    error.value = 'Enter the proxy\'s address and port.'
    return
  }
  busy.value = true
  error.value = await hosts.save({
    id: host.value?.id ?? null,
    label: form.label,
    address: form.address,
    port: form.port === '' ? null : Number(form.port),
    username: form.username,
    password: form.password || null,
    clear_password: clearPassword.value,
    group_id: form.group_id,
    tags: form.tags,
    identity_id: form.identity_id,
    key_id: form.key_id,
    startup_snippet_id: form.startup_snippet_id,
    jump_mode: form.jump_mode,
    jump_host_ids: form.jump_host_ids,
  }, form.terminal_theme, envLoaded.value ? { mode: form.env_mode, vars: form.env_vars.filter((v: EnvVar) => v.name.trim()).map((v: EnvVar) => ({ name: v.name.trim(), value: v.value })) } : undefined, envLoaded.value ? form.proxy : undefined, envLoaded.value ? { kind: form.agent } : undefined)
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
              <div class="relative">
                <NumberInput id="host-port" v-model="form.port" :placeholder="String(inherited.port?.value ?? 22)" />
              </div>
              <p v-if="form.port === '' && inherited.port" class="text-2xs text-muted">from {{ inherited.port.group }}</p>
            </div>
          </div>
          <div class="flex flex-col gap-1.5">
            <Label for="host-group">Group</Label>
            <GroupSelect id="host-group" v-model="form.group_id" :options="hosts.groupOptions()" />
          </div>
          <div class="flex flex-col gap-1.5">
            <Label for="host-tags">Tags</Label>
            <TagInput id="host-tags" v-model="form.tags" :suggestions="hosts.allTags" />
          </div>
        </section>

        <section class="flex flex-col gap-3">
          <h3 class="text-2xs font-medium uppercase tracking-wider text-muted">Credentials</h3>
          <div class="flex flex-col gap-1.5">
            <Label for="host-user">Username</Label>
            <div class="relative">
              <Input id="host-user" v-model="form.username" :placeholder="inherited.username?.value ?? 'deploy'" autocomplete="off" spellcheck="false" :class="inherited.username && !form.username && 'pr-24'" />
              <InheritedHint v-if="inherited.username && !form.username" :group="inherited.username.group" />
            </div>
            <p v-if="!form.username && !inherited.username" class="text-xs text-muted">Leave it blank to be asked when you connect.</p>
          </div>
          <div class="flex flex-col gap-1.5">
            <Label for="host-identity">Identity</Label>
            <OptionSelect id="host-identity" v-model="form.identity_id" :options="identityOptions" none-label="None" />
          </div>
          <div class="flex flex-col gap-1.5">
            <Label for="host-key">Key</Label>
            <OptionSelect id="host-key" v-model="form.key_id" :options="keyOptions" none-label="None" />
            <p v-if="!form.key_id && inherited.key" class="text-xs text-muted">Uses the key {{ keychain.keyLabel(inherited.key.value) ?? 'from a group' }} from {{ inherited.key.group }}.</p>
          </div>
          <div class="flex flex-col gap-1.5">
            <Label for="host-password">Password</Label>
            <PasswordInput id="host-password" v-model="form.password" :saved="hasOwnPassword" />
            <button
              v-if="hasOwnPassword"
              type="button"
              class="self-start text-xs text-muted underline-offset-2 hover:text-text hover:underline"
              @click="clearPassword = true"
            >
              Remove saved password
            </button>
            <p v-else-if="clearPassword" class="text-xs text-muted">The saved password is removed when you save.</p>
            <p v-if="!hasOwnPassword && !form.password && inherited.password" class="text-xs text-muted">Uses the password from {{ inherited.password }}. Type one here to override it.</p>
          </div>
        </section>

        <section class="flex flex-col gap-3">
          <h3 class="text-2xs font-medium uppercase tracking-wider text-muted">Connection</h3>
          <div class="flex flex-col gap-1.5">
            <Label>Jump hosts</Label>
            <ChainPicker
              v-model:mode="form.jump_mode"
              v-model:ids="form.jump_host_ids"
              :options="hosts.hosts.filter(h => h.id !== host?.id)"
              :inherited-ids="inherited.jump?.host_ids ?? []"
              :inherited-from="inherited.jump?.group"
              :target="form.label"
            />
          </div>
        </section>

        <section class="flex flex-col gap-3">
          <h3 class="text-2xs font-medium uppercase tracking-wider text-muted">Terminal</h3>
          <div class="flex flex-col gap-1.5">
            <Label for="host-theme">Theme</Label>
            <ThemeSelect id="host-theme" v-model="form.terminal_theme" :inherited-theme="groupTheme?.inherited" :inherited-from="groupTheme?.inherited_from" />
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
          <h3 class="text-2xs font-medium uppercase tracking-wider text-muted">SSH agent</h3>
          <AgentEditor v-model="form.agent" :info="agentInfo" />
        </section>

        <section class="flex flex-col gap-3">
          <h3 class="text-2xs font-medium uppercase tracking-wider text-muted">On connect</h3>
          <div class="flex flex-col gap-1.5">
            <Label for="host-startup">Startup snippet</Label>
            <OptionSelect id="host-startup" v-model="form.startup_snippet_id" :options="snippetOptions" none-label="None" />
            <p v-if="!form.startup_snippet_id && inherited.startup_snippet" class="text-xs text-muted">Runs {{ snippets.snippetLabel(inherited.startup_snippet.value) ?? 'a snippet' }} from {{ inherited.startup_snippet.group }}.</p>
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
