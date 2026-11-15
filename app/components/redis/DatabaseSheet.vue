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
const environment = computed({ get: () => form.environment as string, set: (v: string) => { form.environment = v as DatabaseForm['environment'] } })
const savedPassword = computed(() => hadPassword.value && !form.clear_password)
const MODES = [
  { value: 'read_only', title: 'Read-only', text: 'Look, never change. Write commands are refused before they are sent.' },
  { value: 'confirm', title: 'Confirm writes', text: 'Every change shows what it will do and asks first.' },
  { value: 'open', title: 'Open', text: 'Edits go straight through. Dangerous commands still ask.' },
] as const

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

        <div class="grid grid-cols-[minmax(0,1fr)_96px] gap-2.5">
          <div class="flex flex-col gap-1.5"><Label for="rdb-address">{{ form.route === 'host' ? 'Address, as seen from the host' : 'Address' }}</Label><Input id="rdb-address" v-model="form.address" class="font-mono text-xs" :placeholder="form.route === 'host' ? '127.0.0.1' : 'redis.example.com'" autocomplete="off" spellcheck="false" /></div>
          <div class="flex flex-col gap-1.5"><Label for="rdb-port">Port</Label><Input id="rdb-port" v-model="form.port" class="font-mono text-xs" inputmode="numeric" autocomplete="off" /></div>
        </div>

        <div class="flex flex-col gap-1.5">
          <Label>Kind</Label>
          <!-- Cluster and Sentinel arrive with their own phase; until then only a single server can be opened. -->
          <div role="radiogroup" aria-label="Kind" class="flex gap-0.5 rounded-md border border-border bg-bg p-0.5">
            <button type="button" role="radio" aria-checked="true" class="grid h-7 flex-1 place-items-center rounded bg-surface-hover px-2 text-xs text-text">Single server</button>
            <button v-for="k in ['Cluster', 'Sentinel']" :key="k" type="button" role="radio" aria-checked="false" disabled title="Not built yet" class="grid h-7 flex-1 place-items-center rounded px-2 text-xs text-muted opacity-60">{{ k }}</button>
          </div>
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
          <div role="radiogroup" aria-labelledby="rdb-writes" class="flex flex-col gap-1.5">
            <button
              v-for="m in MODES"
              :key="m.value"
              type="button"
              role="radio"
              :aria-checked="form.write_mode === m.value"
              :class="cn('flex items-start gap-2.5 rounded-lg border border-border p-2.5 text-left', form.write_mode === m.value && 'border-accent bg-accent/5')"
              @click="form.write_mode = m.value"
            >
              <span :class="cn('mt-[3px] size-3.5 shrink-0 rounded-full border-[1.5px] border-border-input', form.write_mode === m.value && 'border-4 border-accent')" aria-hidden="true" />
              <span class="text-xs text-muted"><b class="text-sm font-medium text-text">{{ m.title }}</b><br>{{ m.text }}</span>
            </button>
          </div>
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
