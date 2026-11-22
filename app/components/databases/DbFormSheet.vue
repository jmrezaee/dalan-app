<script setup lang="ts">
// S35 step 2 of 2: a PostgreSQL, MySQL, MariaDB or MongoDB database: where it is, how to reach it, how to sign in, and
// what Dalan may change in it. The password goes to Rust once and never comes back (only "Saved in the vault").
import { Sheet, SheetContent, SheetDescription, SheetTitle } from '@/components/ui/sheet'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { cn } from '@/lib/utils'
import type { Engine } from '~/types/bindings'
import { blankDb } from '~/stores/databases'
import type { DbForm } from '~/stores/databases'
import { ENGINES } from '~/utils/engines'

const databases = useDatabasesStore()
const hosts = useHostsStore()

const open = computed({
  get: () => databases.editing !== null,
  set: (v: boolean) => { if (!v) { databases.editing = null; databases.pasted = null } },
})
const form = reactive<DbForm>(blankDb('postgres'))
const hadPassword = ref(false)
const password = ref('')
const error = ref('')
const busy = ref(false)
const testing = ref(false)
const tested = ref<{ ok: boolean, text: string } | null>(null)
const isNew = computed(() => !form.id)
const engine = computed(() => form.engine as Engine)
const mongo = computed(() => form.engine === 'mongodb')

watch(() => databases.editing, (e) => {
  if (!e) return
  const saved = 'has_password' in e ? e : null
  Object.assign(form, saved
    ? { ...blankDb(saved.engine as Exclude<Engine, 'redis'>), id: saved.id, name: saved.name, environment: saved.environment, route: saved.route, host_id: saved.host_id, address: saved.address, port: saved.port, default_database: saved.default_database, username: saved.username, tls: saved.tls, tls_server_name: saved.tls_server_name, write_mode: saved.write_mode, options: { ...saved.options } }
    : { ...e })
  hadPassword.value = !!saved?.has_password
  savedMode.value = saved?.environment === 'production' ? saved.write_mode : null
  password.value = ''
  masterPassword.value = ''
  error.value = ''
  tested.value = null
}, { immediate: true })

const hostOptions = computed(() => hosts.hosts.filter(h => !h.protocol || h.protocol === 'ssh' || h.protocol === 'mosh').map(h => ({ value: h.id, label: h.label })))
const hostLabel = computed(() => hosts.hosts.find(h => h.id === form.host_id)?.label ?? 'the host')
const route = computed({
  get: () => form.route as string,
  set: (v: string) => {
    form.route = v as DbForm['route']
    if (v === 'direct') form.host_id = null
    else if (!form.address) form.address = '127.0.0.1'
  },
})
const environment = computed({ get: () => form.environment as string, set: (v: string) => { form.environment = v as DbForm['environment'] } })
const tls = computed({ get: () => form.tls, set: (v: string) => { form.tls = v } })
const savedPassword = computed(() => (hadPassword.value && !form.clear_password) || (!!databases.pasted?.has_password && !password.value))
const addressLabel = computed(() => {
  const seen = form.route === 'host' ? ', as seen from the host' : ''
  return mongo.value ? (form.options?.srv ? `SRV name${seen}` : `Members${seen}`) : `Address${seen}`
})
const addressHint = computed(() => (mongo.value ? (form.options?.srv ? 'cluster0.example.net' : 'db1:27017,db2:27017,db3:27017') : form.route === 'host' ? '127.0.0.1' : 'db.example.com'))
const TLS_NOTE: Record<string, string> = {
  off: 'Nothing is encrypted between Dalan and the database (or the host).',
  require: 'Encrypted, but the certificate is not checked. Use it for a server with a self-made certificate.',
  verify: 'Encrypted, and the certificate must be one this computer trusts.',
}

const RANK = { read_only: 2, confirm: 1, open: 0 } as const
const savedMode = ref<DbForm['write_mode'] | null>(null)
const unmarking = computed(() => savedMode.value !== null && form.environment !== 'production')
const masterPassword = ref('')
watch(unmarking, () => { masterPassword.value = '' })
const lockedModes = computed(() => (savedMode.value && !unmarking.value ? (['read_only', 'confirm', 'open'] as const).filter(m => RANK[m] < RANK[savedMode.value!]) : []))

const input = (): DbForm => ({
  ...form,
  port: mongo.value ? null : (Number(form.port) || 0),
  password: password.value || null,
  options: { ...form.options, auth_source: form.options?.auth_source || null, replica_set: form.options?.replica_set || null },
})

async function test() {
  if (testing.value) return
  testing.value = true
  tested.value = null
  const r = await databases.test(input())
  testing.value = false
  tested.value = r.result ? { ok: true, text: `${r.result.detail} · ${r.result.latency_ms} ms` } : { ok: false, text: r.error }
}

async function submit() {
  if (busy.value) return
  busy.value = true
  if (unmarking.value && !masterPassword.value) {
    busy.value = false
    error.value = 'Type your master password to stop marking this database as production.'
    return
  }
  const r = await databases.save(input(), unmarking.value ? masterPassword.value : null)
  masterPassword.value = ''
  busy.value = false
  error.value = r.error
  if (!r.error) open.value = false
}
</script>

<template>
  <Sheet v-model:open="open">
    <SheetContent class="w-[440px] max-w-full gap-0 p-0 sm:max-w-[440px]" :data-testid="tid('db-form-sheet')">
      <SheetDescription class="sr-only">Where the database is, how to reach it and how to sign in.</SheetDescription>
      <div class="flex h-12 shrink-0 items-center gap-2.5 border-b border-border px-4">
        <EngineBadge :engine="engine" />
        <SheetTitle class="text-base">{{ isNew ? 'New database' : 'Edit database' }}</SheetTitle>
        <span class="text-xs text-muted">{{ isNew ? 'Step 2 of 2 · ' : '' }}{{ ENGINES[engine].label }}</span>
      </div>
      <form class="flex flex-1 flex-col gap-3.5 overflow-auto p-4" @submit.prevent="submit">
        <p v-if="databases.pasted" role="status" class="rounded-md border border-border bg-surface px-3 py-2 text-xs text-muted">Filled from your connection string.{{ databases.pasted.has_password ? ' Its password goes to the vault when you save.' : '' }}</p>
        <div class="flex flex-col gap-1.5"><Label for="db-name">Name</Label><Input id="db-name" v-model="form.name" placeholder="app-prod" autofocus autocomplete="off" spellcheck="false" /></div>

        <div class="flex flex-col gap-1.5">
          <Label>Environment</Label>
          <Segmented v-model="environment" label="Environment" :options="[{ value: 'none', label: 'None' }, { value: 'staging', label: 'Staging' }, { value: 'production', label: 'Production' }]" />
          <div v-if="unmarking" class="flex flex-col gap-1.5 rounded-lg border border-warning/40 p-2.5">
            <label for="db-master-password" class="text-sm font-medium">Master password</label>
            <PasswordInput id="db-master-password" v-model="masterPassword" />
            <p class="text-xs text-muted">This database is marked production. Un-marking it makes it less protected, so it needs your master password.</p>
          </div>
          <p class="text-xs text-muted">Production shows an amber bar on the tab, so nobody mistakes it for staging.</p>
        </div>

        <div class="flex flex-col gap-1.5">
          <Label>Reach it</Label>
          <Segmented v-model="route" label="Reach it" :options="[{ value: 'direct', label: 'Directly' }, { value: 'host', label: 'Through a host' }]" />
        </div>
        <div v-if="form.route === 'host'" class="flex flex-col gap-1.5">
          <Label for="db-host">Host</Label>
          <OptionSelect id="db-host" v-model="form.host_id" :options="hostOptions" none-label="Choose a host" />
          <p v-if="!hostOptions.length" class="text-xs text-muted">No hosts yet. Add one under Hosts first.</p>
          <p v-else class="text-xs text-muted">Dalan signs in to {{ hostLabel }} and talks to {{ ENGINES[engine].label }} from there. Nothing to forward by hand.</p>
        </div>

        <div :class="mongo ? 'flex flex-col gap-1.5' : 'grid grid-cols-[minmax(0,1fr)_96px] gap-2.5'">
          <div class="flex flex-col gap-1.5"><Label for="db-address">{{ addressLabel }}</Label><Input id="db-address" v-model="form.address" class="font-mono text-xs" :placeholder="addressHint" autocomplete="off" spellcheck="false" /></div>
          <div v-if="!mongo" class="flex flex-col gap-1.5"><Label for="db-port">Port</Label><Input id="db-port" :model-value="form.port ?? ''" class="font-mono text-xs" inputmode="numeric" autocomplete="off" @update:model-value="(v: string | number) => { form.port = Number(v) || null }" /></div>
        </div>
        <template v-if="mongo">
          <label class="flex items-center gap-2.5"><Switch aria-label="SRV address" :model-value="form.options?.srv ?? false" @update:model-value="(v: boolean) => { form.options = { ...form.options, srv: v } }" />SRV address<span class="text-xs text-muted">one DNS name that lists the members</span></label>
          <div class="grid grid-cols-2 gap-2.5">
            <div class="flex flex-col gap-1.5"><Label for="db-rs">Replica set (optional)</Label><Input id="db-rs" :model-value="form.options?.replica_set ?? ''" class="font-mono text-xs" placeholder="rs0" autocomplete="off" spellcheck="false" @update:model-value="(v: string | number) => { form.options = { ...form.options, replica_set: String(v) || null } }" /></div>
            <div class="flex flex-col gap-1.5"><Label for="db-auth">Auth source (optional)</Label><Input id="db-auth" :model-value="form.options?.auth_source ?? ''" class="font-mono text-xs" placeholder="admin" autocomplete="off" spellcheck="false" @update:model-value="(v: string | number) => { form.options = { ...form.options, auth_source: String(v) || null } }" /></div>
          </div>
        </template>
        <div class="flex flex-col gap-1.5">
          <Label for="db-database">{{ mongo ? 'Default database (optional)' : 'Database' }}</Label>
          <Input id="db-database" :model-value="form.default_database ?? ''" class="font-mono text-xs" :placeholder="mongo ? 'shop' : 'app'" autocomplete="off" spellcheck="false" @update:model-value="(v: string | number) => { form.default_database = String(v) || null }" />
        </div>

        <div class="grid grid-cols-2 gap-2.5">
          <div class="flex flex-col gap-1.5"><Label for="db-user">Username</Label><Input id="db-user" :model-value="form.username ?? ''" autocomplete="off" spellcheck="false" @update:model-value="(v: string | number) => { form.username = String(v) || null }" /></div>
          <div class="flex flex-col gap-1.5"><Label for="db-pass">Password</Label><PasswordInput id="db-pass" v-model="password" :saved="savedPassword" /></div>
        </div>
        <button v-if="hadPassword && !form.clear_password" type="button" class="-mt-2 self-end text-xs text-muted underline-offset-2 hover:text-text hover:underline" @click="form.clear_password = true">Remove saved password</button>

        <div class="flex flex-col gap-1.5">
          <Label>TLS</Label>
          <Segmented v-model="tls" label="TLS" :options="[{ value: 'off', label: 'Off' }, { value: 'require', label: 'Require' }, { value: 'verify', label: 'Verify' }]" />
          <p class="text-xs text-muted">{{ TLS_NOTE[form.tls] }}</p>
          <div v-if="form.tls === 'verify'" class="flex flex-col gap-1.5"><Label for="db-sni">Certificate name (optional)</Label><Input id="db-sni" :model-value="form.tls_server_name ?? ''" class="font-mono text-xs" placeholder="the address, unless the certificate is for another name" autocomplete="off" spellcheck="false" @update:model-value="(v: string | number) => { form.tls_server_name = String(v) || null }" /></div>
        </div>

        <div class="flex flex-col gap-1.5">
          <Label id="db-writes">Writes</Label>
          <WriteModePicker v-model="form.write_mode" labelledby="db-writes" :locked="lockedModes" />
          <p v-if="lockedModes.length" class="text-xs text-muted">Loosen this from the mode button in its tab (it asks for your master password).</p>
        </div>

        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <button type="submit" class="hidden" tabindex="-1" aria-hidden="true" />
      </form>
      <div class="flex min-h-14 shrink-0 flex-wrap items-center gap-2 border-t border-border px-4 py-2">
        <Button variant="secondary" :loading="testing" @click="test">Test connection</Button>
        <span v-if="tested" role="status" :class="cn('min-w-0 flex-1 text-xs', tested.ok ? 'text-success' : 'text-danger')">{{ tested.text }}</span>
        <span v-else class="flex-1" />
        <Button variant="secondary" @click="open = false">Cancel</Button>
        <Button :loading="busy" @click="submit">Save</Button>
      </div>
    </SheetContent>
  </Sheet>
</template>
