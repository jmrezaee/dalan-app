<script setup lang="ts">
// S03: label, address, port, group, tags, username, password. Empty username, port and password
// inherit from the group, and the field says where from. Jump hosts, env vars and the rest come later.
import { Sheet, SheetContent, SheetDescription, SheetTitle } from '@/components/ui/sheet'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import { ArrowLeft, ChevronRight } from '@lucide/vue'
import { terminalThemeLabels } from '~/utils/terminal-themes'
import type { SerialConfig, AgentConfig, EnvVar, ProxyChoice, HostSummary, InheritedVar } from '~/types/bindings'
const { fixed: shortcutFixed } = useShortcutLabel()

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
/** SSH and Mosh both sign in over SSH, so they share the credential, connection and proxy sections. */
const sshLike = computed(() => form.protocol === 'ssh' || form.protocol === 'mosh')
const serialPorts = ref<string[]>([])
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

const blank = () => ({ label: '', address: '', port: '' as number | string, username: '', password: '', group_id: hosts.currentGroupId as string | null, tags: [] as string[], identity_id: null as string | null, key_id: null as string | null, startup_snippet_id: null as string | null, jump_mode: 'inherit', jump_host_ids: [] as string[], terminal_theme: null as string | null, env_mode: 'inherit', env_vars: [] as EnvVar[], proxy: { kind: 'inherit', host: '', port: null } as ProxyChoice, agent: 'inherit', protocol: 'ssh', serial: { baud: 9600, data_bits: 8, stop_bits: 1, parity: 'none', flow: 'none' } as SerialConfig })
const form = reactive(blank())
// Must come after `form`: a watch reads its getter straight away.
watch(() => form.protocol, async (p) => { if (p === 'serial' && !serialPorts.value.length) serialPorts.value = await api.serialPorts().catch(() => []) })
const clearPassword = ref(false)

// ---- Pages: the main page shows the basics and one line per advanced block; a block opens on its own page with a back arrow.
type Page = 'main' | 'jump' | 'env' | 'proxy' | 'agent' | 'startup' | 'theme'
const page = ref<Page>('main')
const PAGE_TITLES: Record<Exclude<Page, 'main'>, string> = { jump: 'Jump hosts', env: 'Environment variables', proxy: 'Proxy', agent: 'SSH agent', startup: 'On connect', theme: 'Terminal theme' }
const labelOfHost = (id: string) => hosts.hosts.find(h => h.id === id)?.label ?? 'a deleted host'
const summaries = computed<Record<Exclude<Page, 'main'>, string>>(() => {
  const inh = inherited.value
  const jump = form.jump_mode === 'custom' ? (form.jump_host_ids.length ? `Through ${form.jump_host_ids.map(labelOfHost).join(', ')}` : 'None chosen yet') : form.jump_mode === 'none' ? 'Direct' : inh.jump?.host_ids.length ? `From ${inh.jump.group}: ${inh.jump.host_ids.map(labelOfHost).join(', ')}` : 'Direct'
  const named = form.env_vars.filter((v: EnvVar) => v.name.trim()).length
  const env = named ? `${named} ${named === 1 ? 'variable' : 'variables'}${form.env_mode === 'replace' ? ' (replace)' : ''}` : envInherited.value.length ? 'From groups' : 'None'
  const px = form.proxy.kind === 'socks5' || form.proxy.kind === 'http' ? `${form.proxy.kind === 'socks5' ? 'SOCKS5' : 'HTTP'} ${form.proxy.host || '…'}${form.proxy.port ? `:${form.proxy.port}` : ''}` : form.proxy.kind === 'none' ? 'No proxy' : proxyInherited.value?.inherited ? `${proxyInherited.value.inherited} (from group)` : 'None'
  const agent = form.agent === 'inherit' ? 'Same as the group' : form.agent === 'none' || form.agent === 'off' ? 'Off' : 'Forwarded'
  const snippet = form.startup_snippet_id ? (snippets.snippetLabel(form.startup_snippet_id) ?? 'A snippet') : inh.startup_snippet ? `${snippets.snippetLabel(inh.startup_snippet.value) ?? 'A snippet'} (from group)` : 'None'
  return { jump, env, proxy: px, agent, startup: snippet, theme: form.terminal_theme ? (terminalThemeLabels[form.terminal_theme] ?? form.terminal_theme) : 'Default' }
})
const morePages = computed<Exclude<Page, 'main'>[]>(() => (sshLike.value ? ['jump', 'proxy', 'env', 'agent', 'startup', 'theme'] : ['theme']))

// ---- Other hosts that save the same username and password (ids come from Rust; the passwords never do).
const sharedWith = ref<string[]>([])
async function loadShared() {
  sharedWith.value = []
  const id = host.value?.id
  if (!id) return
  try {
    const group = (await api.hostsSharedLogins()).find(g => g.includes(id))
    sharedWith.value = (group ?? []).filter(x => x !== id).map(labelOfHost)
  }
  catch { /* the hint is optional */ }
}
const error = ref('')
const busy = ref(false)
// "Unsaved changes" = the form differs from how it looked when the sheet opened.
const current = computed(() => JSON.stringify([form, clearPassword.value]))
const snapshot = ref('')
const dirty = computed(() => current.value !== snapshot.value)

// Reset the form each time the sheet opens, from the host being edited (or blank, in the group you are viewing).
watch(() => hosts.editing, (e) => {
  const h = e && e !== 'new' ? (e as HostSummary) : null
  Object.assign(form, h ? { label: h.label, address: h.address, port: h.port ?? '', username: h.username ?? '', password: '', group_id: h.group_id, tags: [...h.tags], identity_id: h.identity_id, key_id: h.key_id, startup_snippet_id: h.startup_snippet_id, jump_mode: h.jump_mode, jump_host_ids: [...h.jump_host_ids], terminal_theme: hosts.themes.hosts[h.id]?.own ?? null, env_mode: 'inherit', env_vars: [], proxy: { kind: 'inherit', host: '', port: null }, agent: 'inherit', protocol: h.protocol, serial: { baud: 9600, data_bits: 8, stop_bits: 1, parity: 'none', flow: 'none' } } : blank())
  clearPassword.value = false
  page.value = 'main'
  error.value = ''
  loadShared()
  snapshot.value = current.value
  envInherited.value = []
  proxyInherited.value = null
  agentInfo.value = null
  envLoaded.value = true
  if (h) loadEnv(h.id)
  if (h?.protocol === 'serial') api.serialGet(h.id).then((c) => { form.serial = c; snapshot.value = current.value }).catch(() => {})
})

// The list shows what is being typed as a draft card until it is saved.
watch([() => hosts.editing, () => form.label, () => form.address], () => {
  hosts.draft = hosts.editing === 'new' ? { label: form.label.trim(), address: form.address.trim() } : null
})
onBeforeUnmount(() => { hosts.draft = null })

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
  if (sshLike.value && (form.proxy.kind === 'socks5' || form.proxy.kind === 'http') && (!form.proxy.host.trim() || !form.proxy.port)) {
    error.value = 'Enter the proxy\'s address and port.'
    return
  }
  busy.value = true
  error.value = await hosts.save({
    id: host.value?.id ?? null,
    label: form.label.trim() || form.address.trim(), // the label can be left out: it defaults to the address
    address: form.address,
    port: form.port === '' ? null : Number(form.port),
    username: form.username,
    password: form.protocol !== 'ssh' ? null : form.password || null,
    clear_password: clearPassword.value,
    group_id: form.group_id,
    tags: form.tags,
    identity_id: form.identity_id,
    key_id: form.key_id,
    startup_snippet_id: form.startup_snippet_id,
    jump_mode: form.jump_mode,
    jump_host_ids: form.jump_host_ids,
  }, form.terminal_theme, envLoaded.value ? { mode: form.env_mode, vars: form.env_vars.filter((v: EnvVar) => v.name.trim()).map((v: EnvVar) => ({ name: v.name.trim(), value: v.value })) } : undefined, envLoaded.value && sshLike.value ? form.proxy : undefined, envLoaded.value && sshLike.value ? { kind: form.agent } : undefined, form.protocol, form.serial)
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
        <IconButton v-if="page !== 'main'" label="Back to the host" size="sm" @click="page = 'main'"><ArrowLeft /></IconButton>
        <SheetTitle class="text-base">{{ page === 'main' ? (host ? 'Edit host' : 'New host') : PAGE_TITLES[page] }}</SheetTitle>
        <span v-if="dirty" class="text-xs text-warning">· Unsaved changes</span>
      </div>

      <form class="flex flex-1 flex-col gap-6 overflow-auto p-5" @submit.prevent="submit">
        <template v-if="page === 'main'">
        <section class="flex flex-col gap-3">
          <h3 class="text-2xs font-medium uppercase tracking-wider text-muted">General</h3>
          <Segmented v-model="form.protocol" label="Protocol" :options="[{ value: 'ssh', label: 'SSH' }, { value: 'telnet', label: 'Telnet' }, { value: 'serial', label: 'Serial' }, { value: 'mosh', label: 'Mosh' }]" />
          <p v-if="form.protocol === 'mosh'" role="note" class="rounded-md border border-border bg-surface-raised p-2.5 text-xs text-muted">Mosh keeps a session alive across network changes and sleep. It needs mosh installed here and on the server, can't use jump hosts or a proxy, and talks to the server directly over UDP.</p>
          <p v-if="form.protocol === 'telnet'" role="note" class="rounded-md border border-warning/40 bg-warning/10 p-2.5 text-xs">Telnet isn't encrypted. Everything you type, including passwords, can be read on the network, and Dalan won't save a password for it. Use it only for devices that have nothing better.</p>
          <div :class="form.protocol === 'serial' ? '' : 'grid grid-cols-[3fr_1fr] gap-2.5'">
            <div class="flex flex-col gap-1.5">
              <Label for="host-address">{{ form.protocol === 'serial' ? 'Device' : 'Address' }}</Label>
              <Input id="host-address" v-model="form.address" autofocus :list="form.protocol === 'serial' ? 'serial-ports' : undefined" :placeholder="form.protocol === 'serial' ? '/dev/tty.usbserial-1410 or COM3' : '10.0.0.5'" autocomplete="off" spellcheck="false" class="font-mono" />
              <datalist v-if="form.protocol === 'serial'" id="serial-ports"><option v-for="p in serialPorts" :key="p" :value="p" /></datalist>
            </div>
            <div v-if="form.protocol !== 'serial'" class="flex flex-col gap-1.5">
              <Label for="host-port">Port</Label>
              <div class="relative">
                <NumberInput id="host-port" v-model="form.port" :placeholder="form.protocol === 'telnet' ? '23' : String(inherited.port?.value ?? 22)" />
              </div>
              <p v-if="form.port === '' && inherited.port" class="text-2xs text-muted">from {{ inherited.port.group }}</p>
            </div>
          </div>
          <div class="flex flex-col gap-1.5">
            <Label for="host-label">Label</Label>
            <Input id="host-label" v-model="form.label" placeholder="Same as the address"  autocomplete="off" />
          </div>
          <SerialEditor v-if="form.protocol === 'serial'" v-model="form.serial" />
          <div class="flex flex-col gap-1.5">
            <Label for="host-group">Group</Label>
            <GroupSelect id="host-group" v-model="form.group_id" :options="hosts.groupOptions()" />
          </div>
          <div class="flex flex-col gap-1.5">
            <Label for="host-tags">Tags</Label>
            <TagInput id="host-tags" v-model="form.tags" :suggestions="hosts.allTags" />
          </div>
        </section>

        <section v-if="sshLike" class="flex flex-col gap-3">
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
          <p v-if="sharedWith.length" class="rounded-md border border-border bg-surface-raised p-2.5 text-xs text-muted">Same login as {{ sharedWith.join(', ') }}. An identity in the Keychain keeps it in one place.</p>
        </section>

          <section class="flex flex-col gap-1">
            <h3 class="mb-1 text-2xs font-medium uppercase tracking-wider text-muted">More</h3>
            <button
              v-for="p in morePages"
              :key="p"
              type="button"
              class="flex h-11 items-center gap-3 rounded-md border border-border bg-surface px-3 text-left text-sm hover:bg-surface-hover"
              @click="page = p"
            >
              <span class="w-36 shrink-0">{{ PAGE_TITLES[p] }}</span>
              <span class="min-w-0 flex-1 truncate text-xs text-muted">{{ summaries[p] }}</span>
              <ChevronRight class="size-4 shrink-0 text-muted" aria-hidden="true" />
            </button>
          </section>
        </template>

        <div v-else-if="page === 'jump'" class="flex flex-col gap-1.5">

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
        <div v-else-if="page === 'theme'" class="flex flex-col gap-1.5">

            <Label for="host-theme">Theme</Label>
            <ThemeSelect id="host-theme" v-model="form.terminal_theme" :inherited-theme="groupTheme?.inherited" :inherited-from="groupTheme?.inherited_from" />

        </div>
        <div v-else-if="page === 'startup'" class="flex flex-col gap-1.5">

            <Label for="host-startup">Startup snippet</Label>
            <OptionSelect id="host-startup" v-model="form.startup_snippet_id" :options="snippetOptions" none-label="None" />
            <p v-if="!form.startup_snippet_id && inherited.startup_snippet" class="text-xs text-muted">Runs {{ snippets.snippetLabel(inherited.startup_snippet.value) ?? 'a snippet' }} from {{ inherited.startup_snippet.group }}.</p>

        </div>
        <div v-else-if="page === 'env'">
          <EnvVarsEditor v-model:mode="form.env_mode" v-model:vars="form.env_vars" :inherited="envInherited" />
        </div>
        <div v-else-if="page === 'proxy'">
          <ProxyEditor v-model="form.proxy" :inherited="proxyInherited?.inherited ?? null" :inherited-from="proxyInherited?.inherited_from ?? null" />
        </div>
        <div v-else-if="page === 'agent'">
          <AgentEditor v-model="form.agent" :info="agentInfo" />
        </div>

        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <button type="submit" class="hidden" tabindex="-1" aria-hidden="true" />
      </form>

      <div class="flex h-14 shrink-0 items-center justify-end gap-2 border-t border-border px-5">
        <span class="mr-auto text-xs text-muted"><Kbd>{{ shortcutFixed('Mod+S') }}</Kbd> to save</span>
        <Button variant="secondary" @click="open = false">Cancel</Button>
        <Button :loading="busy" @click="submit">Save</Button>
      </div>
    </SheetContent>
  </Sheet>
</template>
