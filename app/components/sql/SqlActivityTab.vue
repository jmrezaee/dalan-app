<script setup lang="ts">
// S41 "Activity": tiles, sessions, blocking chains and a few server-wide views, refreshed every 5 s while the tab is
// in front. Cancelling a query or ending a session goes through the gate (a confirmation, and on a production
// database the database's name); Dalan's own connections are shown but can't be touched.
import { TriangleAlert } from '@lucide/vue'
import { cn } from '@/lib/utils'
import { Button, buttonVariants } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { ActivityView } from '~/types/bindings'

const props = defineProps<{ tabKey: string, active: boolean }>()
const store = useSqlStore(props.tabKey)
const a = store.activityState
const VIEWS: { id: ActivityView, label: string }[] = [
  { id: 'sessions', label: 'Sessions' }, { id: 'locks', label: 'Locks' }, { id: 'slow', label: 'Slow queries' },
  { id: 'tables', label: 'Tables' }, { id: 'roles', label: 'Roles' }, { id: 'replication', label: 'Replication' },
]
const filter = ref('')
const hideIdle = ref(true)
const now = ref(Date.now())

let timer: ReturnType<typeof setInterval> | undefined
function poll() {
  clearInterval(timer)
  if (!props.active) return
  void store.refreshActivity()
  timer = setInterval(() => { now.value = Date.now(); void store.refreshActivity() }, 5000)
}
watch(() => props.active, poll, { immediate: true })
onBeforeUnmount(() => clearInterval(timer))

const fmt = (n: number) => n.toLocaleString('en-US')
const dur = (s: number | null) => { const t = Math.floor(s ?? 0); return [Math.floor(t / 3600), Math.floor(t / 60) % 60, t % 60].map(x => String(x).padStart(2, '0')).join(':') }
const bytes = (n: number) => (n >= 1 << 30 ? `${(n / (1 << 30)).toFixed(1)} GB` : n >= 1 << 20 ? `${Math.round(n / (1 << 20))} MB` : `${Math.round(n / 1024)} kB`)
const ago = computed(() => (a.at ? Math.max(0, Math.round((now.value - a.at) / 1000)) : null))
const tiles = computed(() => a.data?.tiles ?? null)
const sessions = computed(() => {
  const q = filter.value.trim().toLowerCase()
  return (a.data?.sessions ?? []).filter(s => (!q || `${s.user} ${s.query}`.toLowerCase().includes(q)) && !(hideIdle.value && s.state.startsWith('idle') && !s.state.includes('transaction') && (s.age_s ?? 0) > 60))
})
const STATE_CLASS = (s: string) => (s === 'active' ? 'text-success' : s.includes('transaction') ? 'text-warning' : 'text-muted')

const p = computed(() => a.pending)
const open = computed({ get: () => p.value !== null, set: (v: boolean) => { if (!v) void store.discardSession() } })
const typed = ref('')
watch(p, () => { typed.value = '' })
const typedLevel = computed(() => p.value?.level === 'typed')
const ok = computed(() => !typedLevel.value || typed.value === p.value?.db_name)
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col gap-3 overflow-auto p-3" :data-testid="tid('sql-activity')">
    <div class="flex flex-wrap items-center gap-2">
      <b class="font-medium">Activity</b>
      <span class="text-xs text-muted">{{ ago === null ? 'reading…' : `updated ${ago} s ago` }} · refreshes every 5 s while you look</span>
      <nav class="ml-auto flex gap-0.5" aria-label="Activity views">
        <button v-for="v in VIEWS" :key="v.id" type="button" :aria-current="a.view === v.id ? 'true' : undefined" :class="cn('h-7 cursor-default rounded-md px-2.5 text-sm text-muted outline-none hover:text-text focus-visible:ring-2 focus-visible:ring-accent', a.view === v.id && 'bg-surface-hover text-text')" @click="store.refreshActivity(v.id)">{{ v.label }}</button>
      </nav>
    </div>
    <p v-if="a.error" role="alert" class="text-xs text-danger">{{ a.error }}</p>
    <p v-if="a.note" role="status" class="text-xs text-success" data-part="note">{{ a.note }}</p>

    <div v-if="tiles" class="grid grid-cols-3 gap-2.5 lg:grid-cols-6" data-part="tiles">
      <div class="rounded-lg border border-border bg-surface px-3 py-2"><span class="text-2xs font-medium uppercase tracking-wider text-muted">Connections</span><div class="text-xl font-semibold tabular-nums">{{ fmt(tiles.connections) }}</div><span class="text-xs text-muted">of {{ fmt(tiles.max_connections) }}</span></div>
      <div class="rounded-lg border border-border bg-surface px-3 py-2"><span class="text-2xs font-medium uppercase tracking-wider text-muted">Active</span><div class="text-xl font-semibold tabular-nums">{{ fmt(tiles.active) }}</div><span class="text-xs text-muted">{{ tiles.waiting }} waiting</span></div>
      <div class="rounded-lg border border-border bg-surface px-3 py-2"><span class="text-2xs font-medium uppercase tracking-wider text-muted">Transactions</span><div class="text-xl font-semibold tabular-nums">{{ a.rate === null ? '…' : `${fmt(Math.round(a.rate))} /s` }}</div><span class="text-xs text-muted">now</span></div>
      <div class="rounded-lg border border-border bg-surface px-3 py-2"><span class="text-2xs font-medium uppercase tracking-wider text-muted">Cache hit</span><div class="text-xl font-semibold tabular-nums">{{ tiles.cache_hit_pct === null ? '—' : `${tiles.cache_hit_pct.toFixed(1)}%` }}</div><span class="text-xs text-muted">tables</span></div>
      <div class="rounded-lg border border-border bg-surface px-3 py-2"><span class="text-2xs font-medium uppercase tracking-wider text-muted">Replication lag</span><div class="text-xl font-semibold tabular-nums">{{ tiles.replication_lag_s === null ? '—' : `${tiles.replication_lag_s.toFixed(1)} s` }}</div><span class="text-xs text-muted">{{ tiles.replicas }} {{ tiles.replicas === 1 ? 'replica' : 'replicas' }}</span></div>
      <div class="rounded-lg border border-border bg-surface px-3 py-2"><span class="text-2xs font-medium uppercase tracking-wider text-muted">Database size</span><div class="text-xl font-semibold tabular-nums">{{ bytes(tiles.db_bytes ?? 0) }}</div><span class="text-xs text-muted">now</span></div>
    </div>

    <section v-if="a.view === 'sessions' && a.data" class="rounded-lg border border-border" aria-label="Sessions">
      <div class="flex items-center gap-3 border-b border-border px-3 py-2"><b class="font-medium">Sessions</b><span class="text-xs text-muted">{{ sessions.length }}<template v-if="hideIdle"> · hiding idle ones older than 1 min</template></span>
        <label class="ml-auto flex items-center gap-1.5 text-xs text-muted"><input v-model="hideIdle" type="checkbox">Hide idle</label>
        <Input v-model="filter" class="h-7 w-52 text-xs" placeholder="Filter by user, query" aria-label="Filter sessions" autocomplete="off" />
      </div>
      <table class="w-full table-fixed border-collapse text-xs">
        <thead><tr class="text-left text-muted"><th class="h-7 w-16 px-3 font-medium">PID</th><th class="w-24 font-medium">User</th><th class="w-40 font-medium">State</th><th class="w-20 font-medium">For</th><th class="w-24 font-medium">Wait</th><th class="font-medium">Query</th><th class="w-52" /></tr></thead>
        <tbody>
          <tr v-for="s in sessions" :key="s.pid" :class="cn('border-t border-surface-raised', s.blocked_by.length && 'bg-warning/5', s.state.includes('transaction') && (s.age_s ?? 0) > 120 && 'bg-danger/5')" :data-testid="tid(`session-${s.pid}`)">
            <td class="h-8 px-3 font-mono">{{ s.pid }}</td><td class="truncate">{{ s.mine ? 'you (Dalan)' : s.user }}</td>
            <td :class="STATE_CLASS(s.state)">{{ s.state }}</td><td class="font-mono">{{ dur(s.age_s) }}</td><td class="truncate text-muted">{{ s.wait }}</td>
            <td class="truncate font-mono" :title="s.query">{{ s.query.replace(/\s+/g, ' ') }}</td>
            <td class="pr-2 text-right">
              <template v-if="!s.mine">
                <Button variant="ghost" size="xs" :aria-label="`Cancel query ${s.pid}`" @click="store.prepareSession(s.pid, 'cancel')">Cancel query</Button>
                <Button variant="ghost" size="xs" class="text-danger" :aria-label="`End session ${s.pid}`" @click="store.prepareSession(s.pid, 'end')">End session</Button>
              </template>
            </td>
          </tr>
          <tr v-if="!sessions.length"><td colspan="7" class="p-3 text-muted">Nobody else is connected.</td></tr>
        </tbody>
      </table>
    </section>

    <section v-if="a.view === 'sessions' && a.data?.chains.length" class="rounded-lg border border-danger/45" aria-label="Blocking chains" data-part="chains">
      <div class="flex items-center gap-2 border-b border-border px-3 py-2"><TriangleAlert class="size-3.5 text-danger" aria-hidden="true" /><b class="font-medium">{{ a.data.chains.length }} blocking {{ a.data.chains.length === 1 ? 'chain' : 'chains' }}</b></div>
      <div v-for="c in a.data.chains" :key="c.root.pid" class="flex flex-wrap items-center gap-2.5 p-3 text-xs">
        <span class="rounded-md border border-border bg-bg px-2.5 py-1.5"><span class="font-mono text-danger">{{ c.root.pid }}</span> <span class="text-muted">{{ c.root.user }} · {{ c.root.state }} {{ dur(c.root.age_s) }}</span></span>
        <span class="text-muted">blocks →</span>
        <span v-for="w in c.waiting.slice(0, 3)" :key="w.pid" class="rounded-md border border-border bg-bg px-2.5 py-1.5"><span class="font-mono text-warning">{{ w.pid }}</span> <span class="text-muted">{{ w.user }} · {{ w.query.replace(/\s+/g, ' ').slice(0, 50) }}</span></span>
        <span v-if="c.waiting.length > 3" class="text-muted">{{ c.waiting.length - 3 }} more</span>
        <span class="basis-full text-muted">Ending {{ c.root.pid }} rolls back its open transaction. Dalan asks first, and on production you type the database name.</span>
      </div>
    </section>

    <section v-if="a.view !== 'sessions' && a.data?.table" class="rounded-lg border border-border" :aria-label="VIEWS.find(v => v.id === a.view)?.label">
      <table class="w-full border-collapse text-xs">
        <thead><tr class="text-left text-muted"><th v-for="c in a.data.table.columns" :key="c" class="h-7 px-3 font-medium">{{ c }}</th></tr></thead>
        <tbody>
          <tr v-for="(r, i) in a.data.table.rows" :key="i" class="border-t border-surface-raised"><td v-for="(c, j) in r" :key="j" class="h-8 max-w-96 truncate px-3 font-mono" :title="c">{{ c }}</td></tr>
          <tr v-if="!a.data.table.rows.length"><td :colspan="a.data.table.columns.length" class="p-3 text-muted">Nothing to show.</td></tr>
        </tbody>
      </table>
    </section>

    <AlertDialog v-model:open="open">
      <AlertDialogContent v-if="p" :class="['gap-3 sm:max-w-[520px]', typedLevel && 'border-danger']" :data-testid="tid('session-confirm')">
        <AlertDialogHeader>
          <AlertDialogTitle>{{ p.end ? `End session ${p.pid}?` : `Cancel the query of session ${p.pid}?` }}</AlertDialogTitle>
          <AlertDialogDescription>
            {{ p.end ? 'The person is disconnected and any open transaction is rolled back.' : 'The server stops this statement; the session stays connected.' }}
            <span v-if="p.environment === 'production'" class="text-warning">{{ p.db_name }} is a production database.</span>
          </AlertDialogDescription>
        </AlertDialogHeader>
        <dl class="grid grid-cols-[64px_minmax(0,1fr)] gap-x-2.5 gap-y-1 text-xs"><dt class="text-muted">User</dt><dd>{{ p.user }}</dd><dt class="text-muted">State</dt><dd>{{ p.state }}</dd><dt class="text-muted">Query</dt><dd class="break-all font-mono">{{ p.query }}</dd></dl>
        <label v-if="typedLevel" class="flex flex-col gap-1 text-xs text-muted">Type {{ p.db_name }} to confirm
          <Input v-model="typed" class="font-mono text-xs text-text" autocomplete="off" spellcheck="false" @keydown.enter.prevent="ok && store.confirmSession(typed)" />
        </label>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <Button :class="buttonVariants({ variant: 'destructive' })" :disabled="!ok" :loading="a.busy" @click="store.confirmSession(typedLevel ? typed : null)">{{ p.end ? 'End session' : 'Cancel query' }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
