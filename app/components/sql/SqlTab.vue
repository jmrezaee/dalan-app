<script setup lang="ts">
// A SQL tab (S36 shell and S33 states for PostgreSQL): the header, connecting, the host's questions, failures, the
// object tree and the open tables. The tab stays mounted while another tab shows, so its connection and tables carry on.
import { CircleAlert, Lock, ShieldCheck, Unlock, X } from '@lucide/vue'
import { cn } from '@/lib/utils'
import { ENGINES } from '~/utils/engines'

const props = defineProps<{ tabKey: string, dbId: string, active: boolean }>()
const emit = defineEmits<{ close: [] }>()
const store = useSqlStore(props.tabKey)
const databases = useDatabasesStore()
const hosts = useHostsStore()

const db = computed(() => databases.rows.find(d => d.id === props.dbId) ?? null)
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
const version = computed(() => store.info?.version.split(' ').slice(0, 2).join(' ') ?? '')

const failure = computed(() => {
  const e = store.error
  if (store.status !== 'failed' || !e) return null
  const edit = { label: 'Edit database', run: () => { if (db.value) databases.editing = db.value } }
  const again = { label: 'Try again', run: () => store.retry() }
  if (e.code === 'db_auth_failed') return { title: 'Wrong username or password', text: e.message, action: edit }
  if (e.code === 'db_tls_untrusted') return { title: 'This server\'s certificate isn\'t trusted', text: e.message, action: edit }
  if (e.code === 'db_unreachable') return { title: `Can't reach ${db.value ? ENGINES[db.value.engine].label : 'the database'}`, text: e.message, action: again }
  if (e.code === 'not_found') return { title: 'This database is no longer saved', text: 'It was deleted. Close this tab.', action: { label: 'Close tab', run: () => emit('close') } }
  return { title: 'Couldn\'t open this database', text: e.message, action: e.code === 'validation' ? edit : again }
})
const promptHost = computed(() => {
  const k = store.prompt?.kind
  return k && 'host' in k ? k.host : (db.value?.host_label ?? '')
})

function onTabKey(e: KeyboardEvent, i: number) {
  const d = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : 0
  if (!d) return
  e.preventDefault()
  const next = store.tabs[(i + d + store.tabs.length) % store.tabs.length]
  if (next) { store.activeTab = next.id; nextTick(() => document.getElementById(`${props.tabKey}-tab-${next.id}`)?.focus()) }
}
</script>

<template>
  <div class="flex size-full flex-col bg-bg" :data-testid="tid(`sql-tab-${name}`)">
    <header class="flex h-11 shrink-0 items-center gap-3 border-b border-border px-4">
      <EngineBadge v-if="db" :engine="db.engine" />
      <h1 class="text-sm font-medium">{{ name }}</h1>
      <span v-if="db && db.environment !== 'none'" :class="cn('inline-flex h-5 items-center rounded-full border px-2 text-2xs font-medium', db.environment === 'production' ? 'border-warning/40 text-warning' : 'border-border text-muted')">{{ db.environment }}</span>
      <span class="truncate text-xs text-muted">{{ routeText }}<template v-if="db"> → {{ db.address }}:{{ db.port }}</template></span>
      <span v-if="store.info" class="truncate text-xs text-muted">· {{ store.info.database }} · {{ version }}</span>
      <span v-if="store.info" class="ml-auto flex items-center gap-1.5 text-xs text-muted" title="How long the first answer took"><span class="size-2 rounded-full bg-success" aria-hidden="true" />{{ store.info.latency_ms }} ms</span>
      <span v-else class="ml-auto" />
      <span class="inline-flex h-7 items-center gap-1.5 rounded-md border border-border px-2 text-xs text-muted" :title="'Writes: ' + mode.label"><component :is="mode.icon" class="size-3.5" aria-hidden="true" />{{ mode.label }}</span>
    </header>

    <AlertDialog v-for="q in store.queries.filter(x => x.closeAsk)" :key="q.id" :open="true" @update:open="(v: boolean) => { if (!v) q.closeAsk = false }">
      <AlertDialogContent :data-testid="tid('sql-close-guard')">
        <AlertDialogHeader>
          <AlertDialogTitle>Close {{ q.title }} with a transaction open?</AlertDialogTitle>
          <AlertDialogDescription>{{ q.txn?.statements ?? 0 }} {{ q.txn?.statements === 1 ? 'statement is' : 'statements are' }} not committed. Commit keeps them; Roll back undoes them.</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Keep it open</AlertDialogCancel>
          <Button variant="secondary" @click="store.finishAndClose(q, false)">Roll back and close</Button>
          <Button :disabled="q.txn?.failed" @click="store.finishAndClose(q, true)">Commit and close</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>

    <div v-if="store.status === 'ready'" class="flex min-h-0 flex-1">
      <SqlTree :tab-key="tabKey" />
      <section class="flex min-w-0 flex-1 flex-col" aria-label="Tables">
        <div role="group" aria-label="Open tabs" class="flex h-9 shrink-0 overflow-x-auto border-b border-border">
          <div v-for="(t, i) in store.tabs" :key="t.id" :class="cn('flex shrink-0 items-center border-r border-border text-sm text-muted', store.activeTab === t.id && 'bg-bg text-text shadow-[inset_0_2px_0_var(--color-accent)]')">
            <button
              :id="`${tabKey}-tab-${t.id}`" type="button" :aria-current="store.activeTab === t.id ? 'true' : undefined"
              class="h-full cursor-default pl-3 pr-1 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" @click="store.activeTab = t.id" @keydown="onTabKey($event, i)"
            >
              {{ t.table }}
            </button>
            <button type="button" class="mr-1.5 grid size-5 place-items-center rounded hover:bg-surface-hover" :aria-label="`Close ${t.table}`" @click="store.closeTable(t.id)"><X class="size-3" aria-hidden="true" /></button>
          </div>
          <div v-for="k in store.special" :key="k" :class="cn('flex shrink-0 items-center border-r border-border text-sm text-muted', store.activeTab === k && 'bg-bg text-text shadow-[inset_0_2px_0_var(--color-accent)]')">
            <button :id="`${tabKey}-tab-${k}`" type="button" :aria-current="store.activeTab === k ? 'true' : undefined" class="h-full cursor-default pl-3 pr-1 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" @click="store.activeTab = k">{{ k === 'activity' ? 'Activity' : 'Transfer' }}</button>
            <button type="button" class="mr-1.5 grid size-5 place-items-center rounded hover:bg-surface-hover" :aria-label="`Close ${k === 'activity' ? 'Activity' : 'Transfer'}`" @click="store.closeSpecial(k)"><X class="size-3" aria-hidden="true" /></button>
          </div>
          <div v-for="g in store.diagrams" :key="g.id" :class="cn('flex shrink-0 items-center border-r border-border text-sm text-muted', store.activeTab === g.id && 'bg-bg text-text shadow-[inset_0_2px_0_var(--color-accent)]')">
            <button :id="`${tabKey}-tab-${g.id}`" type="button" :aria-current="store.activeTab === g.id ? 'true' : undefined" class="flex h-full cursor-default items-center gap-1.5 pl-3 pr-1 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" @click="store.activeTab = g.id">{{ g.title }}</button>
            <button type="button" class="mr-1.5 grid size-5 place-items-center rounded hover:bg-surface-hover" :aria-label="`Close ${g.title}`" @click="store.closeDiagram(g.id)"><X class="size-3" aria-hidden="true" /></button>
          </div>
          <div v-for="q in store.queries" :key="q.id" :class="cn('flex shrink-0 items-center border-r border-border text-sm text-muted', store.activeTab === q.id && 'bg-bg text-text shadow-[inset_0_2px_0_var(--color-accent)]')">
            <button
              :id="`${tabKey}-tab-${q.id}`" type="button" :aria-current="store.activeTab === q.id ? 'true' : undefined"
              class="flex h-full cursor-default items-center gap-1.5 pl-3 pr-1 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" @click="store.activeTab = q.id"
            >
              {{ q.title }}<span v-if="q.text !== q.saved" class="size-1.5 rounded-full bg-accent" aria-label="unsaved" />
            </button>
            <button type="button" class="mr-1.5 grid size-5 place-items-center rounded hover:bg-surface-hover" :aria-label="`Close ${q.title}`" @click="store.requestCloseQuery(q)"><X class="size-3" aria-hidden="true" /></button>
          </div>
        </div>
        <template v-for="t in store.tabs" :key="t.id">
          <SqlTableView v-show="store.activeTab === t.id" :tab-key="tabKey" :tab="t" />
        </template>
        <!-- Query tabs stay mounted while another tab shows, so their editors keep the caret and the undo history. -->
        <SqlActivityTab v-if="store.special.includes('activity')" v-show="store.activeTab === 'activity'" :tab-key="tabKey" :active="active && store.activeTab === 'activity'" />
        <SqlTransferTab v-if="store.special.includes('transfer')" v-show="store.activeTab === 'transfer'" :tab-key="tabKey" />
        <SqlDiagramTab v-for="g in store.diagrams" v-show="store.activeTab === g.id" :key="g.id" :tab-key="tabKey" :diagram="g" />
        <SqlQueryTab v-for="q in store.queries" v-show="store.activeTab === q.id" :key="q.id" :tab-key="tabKey" :query="q" />
        <EmptyState v-if="!store.tabs.length && !store.queries.length && !store.diagrams.length && !store.special.length" :icon="ShieldCheck" title="Open a table or start a query" text="Pick a table from the list, or press New query. Only reads run for now: changes come with the next update." />
      </section>
    </div>

    <main v-else class="grid min-h-0 flex-1 place-items-center overflow-auto p-6">
      <div v-if="store.prompt" class="w-[420px] max-w-full rounded-lg border border-border bg-surface p-4" role="alertdialog" :aria-label="`${promptHost} needs an answer`">
        <h2 class="mb-1 text-base font-semibold">Opening {{ name }}</h2>
        <p class="mb-3 text-sm text-muted">{{ promptHost }} needs an answer before Dalan can reach the database.</p>
        <PromptPanel :kind="store.prompt.kind" :host="promptHost" @reply="store.respond" />
      </div>
      <ConnectSteps v-else-if="store.status === 'connecting'" :name="name" :steps="store.steps" @cancel="emit('close')" />
      <div v-else-if="failure" class="flex w-[460px] max-w-full items-center gap-2.5 rounded-lg border border-border bg-surface px-3 py-2.5" role="alert" :data-testid="tid('sql-failed')">
        <CircleAlert class="size-4 shrink-0 text-danger" aria-hidden="true" />
        <span class="min-w-0 flex-1"><b class="font-medium">{{ failure.title }}</b><br><span class="text-xs text-muted">{{ failure.text }}</span></span>
        <Button variant="secondary" size="sm" data-autofocus @click="failure.action.run()">{{ failure.action.label }}</Button>
      </div>
      <div v-else-if="store.status === 'closed'" class="flex w-[460px] max-w-full items-center gap-2.5 rounded-lg border border-border bg-surface px-3 py-2.5" role="status">
        <CircleAlert class="size-4 shrink-0 text-danger" aria-hidden="true" />
        <span class="min-w-0 flex-1"><b class="font-medium">Connection closed</b><br><span class="text-xs text-muted">The server hung up, or the host's connection ended.</span></span>
        <Button variant="secondary" size="sm" @click="store.retry()">Reconnect</Button>
      </div>
    </main>
  </div>
</template>
