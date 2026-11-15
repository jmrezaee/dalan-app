<script setup lang="ts">
// A Redis tab (replica/redis/spec.md, S28 shell and S33 states). R1 builds the shell: the toolbar, connecting,
// the host's questions, failures and drops. The views themselves (Browser, Console, Pub/Sub, Server) arrive in R2 to R5.
// The tab stays mounted while another tab shows, so its connection and state carry on.
import { CircleAlert, Database, Lock, ShieldCheck, Unlock } from '@lucide/vue'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { cn } from '@/lib/utils'

const props = defineProps<{ tabKey: string, dbId: string, active: boolean }>()
const emit = defineEmits<{ close: [] }>()
const store = useRedisStore(props.tabKey)
const databases = useRedisDatabasesStore()
const hosts = useHostsStore()

const db = computed(() => databases.dbs.find(d => d.id === props.dbId) ?? null)
const name = computed(() => db.value?.name ?? 'this database')
const routeText = computed(() => {
  const d = db.value
  if (!d || d.route === 'direct') return 'Direct'
  const host = hosts.hosts.find(h => h.id === d.host_id)
  const via = (host?.effective_jump_host_ids ?? []).map(id => hosts.hosts.find(h => h.id === id)?.label).filter(Boolean)
  return [...via, d.host_label ?? 'a host'].join(' → ')
})
const MODE = { read_only: { label: 'Read-only', icon: Lock }, confirm: { label: 'Confirm writes', icon: ShieldCheck }, open: { label: 'Open', icon: Unlock } } as const
const mode = computed(() => MODE[db.value?.write_mode ?? 'confirm'])

const VIEWS = [{ id: 'browser', label: 'Browser' }, { id: 'console', label: 'Console' }, { id: 'pubsub', label: 'Pub/Sub' }, { id: 'server', label: 'Server' }] as const
function onViewKey(e: KeyboardEvent, i: number) {
  const d = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : 0
  if (!d) return
  e.preventDefault()
  const next = VIEWS[(i + d + VIEWS.length) % VIEWS.length]!
  store.view = next.id
  nextTick(() => document.getElementById(`${props.tabKey}-view-${next.id}`)?.focus())
}

// The logical database picker: every database the server has, with its key count when it holds any.
const dbOptions = computed(() => Array.from({ length: store.info?.db_count ?? 0 }, (_, i) => ({ index: i, keys: store.keys[i] ?? 0 })))
const count = (n: number) => n.toLocaleString('en-US')
const latencyTone = computed(() => (store.latencyMs === null ? 'bg-border-input' : store.latencyMs < 100 ? 'bg-success' : store.latencyMs < 500 ? 'bg-warning' : 'bg-danger'))

// One line of what happened and one useful action (S33 "When it fails").
const failure = computed(() => {
  const e = store.error
  if (store.status !== 'failed' || !e) return null
  const edit = { label: 'Edit database', run: () => { if (db.value) databases.editing = db.value } }
  const again = { label: 'Try again', run: () => store.retry() }
  if (e.code === 'redis_auth_failed') return { title: 'Wrong username or password', text: 'Redis refused the sign-in.', action: edit }
  if (e.code === 'redis_tls_untrusted') return { title: 'This server\'s certificate isn\'t trusted', text: e.message, action: edit }
  if (e.code === 'redis_unreachable') return { title: 'Can\'t reach Redis', text: e.message, action: again }
  if (e.code === 'not_found') return { title: 'This database is no longer saved', text: 'It was deleted. Close this tab.', action: { label: 'Close tab', run: () => emit('close') } }
  return { title: 'Couldn\'t open this database', text: e.message, action: e.code === 'validation' ? edit : again }
})
const promptHost = computed(() => {
  const k = store.prompt?.kind
  return k && 'host' in k ? k.host : (db.value?.host_label ?? '')
})
</script>

<template>
  <div class="flex size-full flex-col bg-bg" :data-testid="tid(`redis-tab-${name}`)">
    <header class="flex h-11 shrink-0 items-center gap-3 border-b border-border px-4">
      <Database class="size-4 shrink-0 text-muted" aria-hidden="true" />
      <h1 class="text-sm font-medium">{{ name }}</h1>
      <span v-if="db && db.environment !== 'none'" :class="cn('inline-flex h-5 items-center rounded-full border px-2 text-2xs font-medium', db.environment === 'production' ? 'border-warning/40 text-warning' : 'border-border text-muted')">{{ db.environment }}</span>
      <span class="truncate text-xs text-muted">{{ routeText }}</span>

      <Select v-if="store.status === 'ready' && store.info && store.info.mode !== 'cluster'" :model-value="String(store.selected)" @update:model-value="(v) => store.select(Number(v))">
        <SelectTrigger class="h-7 w-auto gap-1.5 px-2 font-mono text-xs" aria-label="Logical database"><SelectValue>db{{ store.selected }} · {{ count(store.keys[store.selected] ?? 0) }} keys</SelectValue></SelectTrigger>
        <SelectContent><SelectItem v-for="o in dbOptions" :key="o.index" :value="String(o.index)" class="font-mono text-xs">db{{ o.index }} · {{ count(o.keys) }} keys</SelectItem></SelectContent>
      </Select>

      <div role="tablist" aria-label="Views" class="ml-3 flex gap-0.5">
        <button
          v-for="(v, i) in VIEWS"
          :id="`${tabKey}-view-${v.id}`"
          :key="v.id"
          type="button"
          role="tab"
          :aria-selected="store.view === v.id"
          :tabindex="store.view === v.id ? 0 : -1"
          :class="cn('h-7 rounded-md px-2.5 text-sm text-muted hover:text-text', store.view === v.id && 'bg-surface-hover text-text')"
          @click="store.view = v.id"
          @keydown="onViewKey($event, i)"
        >
          {{ v.label }}
        </button>
      </div>

      <span class="ml-auto flex items-center gap-1.5 text-xs text-muted" :title="store.latencyMs === null ? 'Not connected' : 'How long the server takes to answer'">
        <span :class="cn('size-2 rounded-full', latencyTone)" aria-hidden="true" />{{ store.latencyMs === null ? '…' : `${store.latencyMs} ms` }}
      </span>
      <!-- The mode is changed in the editor for now; changing it here, with the master password on production, arrives with the write gate (R3). -->
      <Button variant="secondary" size="sm" :title="'Writes: ' + mode.label + '. Change it under Edit database.'" @click="db && (databases.editing = db)"><component :is="mode.icon" />{{ mode.label }}</Button>
    </header>

    <div v-if="store.status === 'ready' && store.info?.replica" class="flex shrink-0 items-center gap-2 border-b border-border bg-surface-raised px-4 py-2 text-sm" role="status">
      <Lock class="size-3.5 text-muted" aria-hidden="true" />This server is a replica. It only accepts reads. Writes need the primary.
    </div>
    <div v-if="store.status === 'dropped' && store.dropped" class="flex shrink-0 items-center gap-2.5 border-b border-border px-4 py-2" role="status" :data-testid="tid('redis-dropped')">
      <CircleAlert class="size-4 text-danger" aria-hidden="true" />
      <span class="flex-1"><b class="font-medium">The connection dropped</b><br><span class="text-xs text-muted">Trying again in {{ store.dropped.retryInSecs }} s ({{ store.dropped.attempt }} of {{ store.dropped.of }}).</span></span>
      <Button variant="secondary" size="sm" @click="store.retry()">Reconnect now</Button>
    </div>

    <!-- Connected: the open view. The Browser is built; the other three arrive in R4 and R5. -->
    <RedisBrowser v-if="store.status === 'ready' && store.view === 'browser' && !store.prompt" :tab-key="tabKey" :separator="db?.key_separator ?? ':'" />

    <main v-else class="grid min-h-0 flex-1 place-items-center overflow-auto p-6">
      <!-- A hop's question (host key, password, passphrase, 2FA): the same prompts a terminal shows. -->
      <div v-if="store.prompt" class="w-[420px] max-w-full rounded-lg border border-border bg-surface p-4" role="alertdialog" :aria-label="`${promptHost} needs an answer`">
        <h2 class="mb-1 text-base font-semibold">Opening {{ name }}</h2>
        <p class="mb-3 text-sm text-muted">{{ promptHost }} needs an answer before Dalan can reach Redis.</p>
        <PromptPanel :kind="store.prompt.kind" :host="promptHost" @reply="store.respond" />
      </div>

      <ConnectSteps v-else-if="store.status === 'connecting' || (store.status === 'dropped' && store.steps.length)" :name="name" :steps="store.steps" @cancel="emit('close')" />

      <div v-else-if="failure" class="flex w-[460px] max-w-full items-center gap-2.5 rounded-lg border border-border bg-surface px-3 py-2.5" role="alert" :data-testid="tid('redis-failed')">
        <CircleAlert class="size-4 shrink-0 text-danger" aria-hidden="true" />
        <span class="min-w-0 flex-1"><b class="font-medium">{{ failure.title }}</b><br><span class="text-xs text-muted">{{ failure.text }}</span></span>
        <Button variant="secondary" size="sm" data-autofocus @click="failure.action.run()">{{ failure.action.label }}</Button>
      </div>

      <!-- Console, Pub/Sub and Server arrive in the next phases. Until then they say so, honestly. -->
      <EmptyState v-else-if="store.status === 'ready' && store.info" :icon="Database" :title="`Connected to ${name}`" :text="`Redis ${store.info.version}${store.info.resp3 ? '' : ' (older protocol)'}. The ${VIEWS.find(v => v.id === store.view)?.label} view isn't built yet.`" />
    </main>
  </div>
</template>
