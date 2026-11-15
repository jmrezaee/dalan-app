<script setup lang="ts">
// S27: a Redis database: where it is, how to reach it, how to sign in, and what Dalan may change in it.
// The password goes to Rust once and never comes back (only "Saved in the vault").
import { Sheet, SheetContent, SheetDescription, SheetTitle } from '@/components/ui/sheet'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { cn } from '@/lib/utils'
import { blankDatabase } from '~/stores/redisDatabases'
import type { DatabaseForm } from '~/stores/redisDatabases'

const databases = useRedisDatabasesStore()
const hosts = useHostsStore()

const open = computed({
  get: () => databases.editing !== null,
  set: (v: boolean) => { if (!v) databases.editing = null },
})
const form = reactive<DatabaseForm>(blankDatabase())
const hadPassword = ref(false)
const password = ref('')
const error = ref('')
const busy = ref(false)
const testing = ref(false)
const tested = ref<{ ok: boolean, text: string } | null>(null)
const isNew = computed(() => !form.id)

watch(() => databases.editing, (e) => {
  if (!e) return
  // A saved database has `has_password`; a new one is already the form's shape.
  const saved = 'has_password' in e ? e : null
  Object.assign(form, saved
    ? { ...blankDatabase(), id: saved.id, name: saved.name, environment: saved.environment, route: saved.route, host_id: saved.host_id, address: saved.address, port: saved.port, kind: saved.kind, sentinel_master: saved.sentinel_master, username: saved.username, tls: saved.tls, tls_server_name: saved.tls_server_name, write_mode: saved.write_mode, default_db: saved.default_db, key_separator: saved.key_separator }
    : { ...e })
  hadPassword.value = !!saved?.has_password
  savedMode.value = saved?.environment === 'production' ? saved.write_mode : null
  password.value = ''
  error.value = ''
  tested.value = null
}, { immediate: true })

// Only SSH hosts can carry a connection to Redis.
const hostOptions = computed(() => hosts.hosts.filter(h => !h.protocol || h.protocol === 'ssh' || h.protocol === 'mosh').map(h => ({ value: h.id, label: h.label })))
const hostLabel = computed(() => hosts.hosts.find(h => h.id === form.host_id)?.label ?? 'the host')
const route = computed({
  get: () => form.route as string,
  set: (v: string) => {
    form.route = v as DatabaseForm['route']
    if (v === 'direct') form.host_id = null
    else if (!form.address) form.address = '127.0.0.1'
  },
})
// Switching the kind moves the port to that kind's usual one, unless it was changed by hand.
const USUAL_PORT = { single: 6379, cluster: 6379, sentinel: 26379 } as const
const kind = computed({
  get: () => form.kind as string,
  set: (v: string) => {
    const next = v as DatabaseForm['kind']
    if (Number(form.port) === USUAL_PORT[form.kind]) form.port = USUAL_PORT[next]
    form.kind = next
    if (next !== 'sentinel') form.sentinel_master = null
  },
})
const addressLabel = computed(() => {
  const seen = form.route === 'host' ? ', as seen from the host' : ''
  return form.kind === 'sentinel' ? `Sentinels${seen}` : form.kind === 'cluster' ? `Address of any node${seen}` : `Address${seen}`
})
const environment = computed({ get: () => form.environment as string, set: (v: string) => { form.environment = v as DatabaseForm['environment'] } })
const savedPassword = computed(() => hadPassword.value && !form.clear_password)
// A saved production database can't be loosened here: that goes through the tab's mode button, which asks for the
// master password (Rust refuses it from this form either way).
const RANK = { read_only: 2, confirm: 1, open: 0 } as const
const savedMode = ref<DatabaseForm['write_mode'] | null>(null)
const lockedModes = computed(() => (savedMode.value ? (['read_only', 'confirm', 'open'] as const).filter(m => RANK[m] < RANK[savedMode.value!]) : []))

const input = (): DatabaseForm => ({ ...form, port: Number(form.port) || 0, password: password.value || null })

async function test() {
  if (testing.value) return
  testing.value = true
  tested.value = null
  const r = await databases.test(input())
  testing.value = false
  tested.value = r.result
    ? { ok: true, text: `Connected · Redis ${r.result.version} · ${r.result.latency_ms} ms${r.result.replica ? ' · replica' : ''}` }
    : { ok: false, text: r.error }
}

async function submit() {
  if (busy.value) return
  busy.value = true
  const r = await databases.save(input())
  busy.value = false
  error.value = r.error
  if (!r.error) open.value = false
}
</script>

<template>
  <Sheet v-model:open="open">
    <SheetContent class="w-[440px] max-w-full gap-0 p-0 sm:max-w-[440px]" :data-testid="tid('database-sheet')">
      <SheetDescription class="sr-only">Where the database is, how to reach it and how to sign in.</SheetDescription>
      <div class="flex h-12 shrink-0 items-center border-b border-border px-4"><SheetTitle class="text-base">{{ isNew ? 'New database' : 'Edit database' }}</SheetTitle></div>
      <form class="flex flex-1 flex-col gap-3.5 overflow-auto p-4" @submit.prevent="submit">
        <div class="flex flex-col gap-1.5"><Label for="rdb-name">Name</Label><Input id="rdb-name" v-model="form.name" placeholder="cache-prod" autofocus autocomplete="off" spellcheck="false" /></div>

        <div class="flex flex-col gap-1.5">
          <Label>Environment</Label>
          <Segmented v-model="environment" label="Environment" :options="[{ value: 'none', label: 'None' }, { value: 'staging', label: 'Staging' }, { value: 'production', label: 'Production' }]" />
          <p class="text-xs text-muted">Production shows an amber bar on the tab, so nobody mistakes it for staging.</p>
        </div>

        <div class="flex flex-col gap-1.5">
          <Label>Reach it</Label>
          <Segmented v-model="route" label="Reach it" :options="[{ value: 'direct', label: 'Directly' }, { value: 'host', label: 'Through a host' }]" />
        </div>
        <div v-if="form.route === 'host'" class="flex flex-col gap-1.5">
          <Label for="rdb-host">Host</Label>
          <OptionSelect id="rdb-host" v-model="form.host_id" :options="hostOptions" none-label="Choose a host" />
          <p v-if="!hostOptions.length" class="text-xs text-muted">No hosts yet. Add one under Hosts first.</p>
          <p v-else class="text-xs text-muted">Dalan signs in to {{ hostLabel }} and talks to Redis from there. Nothing to forward by hand.</p>
        </div>

        <div class="flex flex-col gap-1.5">
          <Label id="rdb-kind">Kind</Label>
          <Segmented v-model="kind" label="Kind" :options="[{ value: 'single', label: 'Single server' }, { value: 'cluster', label: 'Cluster' }, { value: 'sentinel', label: 'Sentinel' }]" />
          <p v-if="form.kind === 'cluster'" class="text-xs text-muted">Give the address of any node. Dalan asks it where the others are and reaches each one the same way.</p>
          <p v-else-if="form.kind === 'sentinel'" class="text-xs text-muted">List the sentinels. Dalan asks them where the primary is, and asks again if it moves.</p>
        </div>

        <div class="grid grid-cols-[minmax(0,1fr)_96px] gap-2.5">
          <div class="flex flex-col gap-1.5"><Label for="rdb-address">{{ addressLabel }}</Label><Input id="rdb-address" v-model="form.address" class="font-mono text-xs" :placeholder="form.kind === 'sentinel' ? '10.0.2.21:26379, 10.0.2.22:26379' : form.route === 'host' ? '127.0.0.1' : 'redis.example.com'" autocomplete="off" spellcheck="false" /></div>
          <div class="flex flex-col gap-1.5"><Label for="rdb-port">{{ form.kind === 'sentinel' ? 'Default port' : 'Port' }}</Label><Input id="rdb-port" v-model="form.port" class="font-mono text-xs" inputmode="numeric" autocomplete="off" /></div>
        </div>
        <div v-if="form.kind === 'sentinel'" class="flex flex-col gap-1.5">
          <Label for="rdb-master">Primary's name</Label>
          <Input id="rdb-master" :model-value="form.sentinel_master ?? ''" class="font-mono text-xs" placeholder="mymaster" autocomplete="off" spellcheck="false" @update:model-value="(v: string | number) => { form.sentinel_master = String(v) || null }" />
          <p class="text-xs text-muted">The name the sentinels know the primary by.</p>
        </div>

        <div class="grid grid-cols-2 gap-2.5">
          <div class="flex flex-col gap-1.5"><Label for="rdb-user">Username (optional)</Label><Input id="rdb-user" :model-value="form.username ?? ''" autocomplete="off" spellcheck="false" @update:model-value="(v: string | number) => { form.username = String(v) || null }" /></div>
          <div class="flex flex-col gap-1.5">
            <Label for="rdb-pass">Password</Label>
            <PasswordInput id="rdb-pass" v-model="password" :saved="savedPassword" />
          </div>
        </div>
        <button v-if="savedPassword" type="button" class="-mt-2 self-end text-xs text-muted underline-offset-2 hover:text-text hover:underline" @click="form.clear_password = true">Remove saved password</button>

        <div class="flex flex-col gap-1.5">
          <label class="flex items-center gap-2.5"><Switch aria-label="Use TLS" :model-value="form.tls" @update:model-value="(v: boolean) => { form.tls = v }" />Use TLS<span class="text-xs text-muted">certificate is checked</span></label>
          <div v-if="form.tls" class="flex flex-col gap-1.5"><Label for="rdb-sni">Certificate name (optional)</Label><Input id="rdb-sni" :model-value="form.tls_server_name ?? ''" class="font-mono text-xs" placeholder="the address, unless the certificate is for another name" autocomplete="off" spellcheck="false" @update:model-value="(v: string | number) => { form.tls_server_name = String(v) || null }" /></div>
        </div>

        <div class="flex flex-col gap-1.5">
          <Label id="rdb-writes">Writes</Label>
          <WriteModePicker v-model="form.write_mode" labelledby="rdb-writes" :locked="lockedModes" />
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
