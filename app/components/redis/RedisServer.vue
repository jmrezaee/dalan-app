<script setup lang="ts">
// S31: what the server says about itself (INFO) and its own slow log. Read-only, except "Clear log…", which is a
// dangerous command and goes through the gate like one. Dalan never turns on live command tracing.
import { CircleAlert, RefreshCw } from '@lucide/vue'
import { count, sizeText, ttlText } from '~/utils/redisBytes'

const props = defineProps<{ tabKey: string, active: boolean, readOnly: boolean }>()
const store = useRedisStore(props.tabKey)

// Refresh every 5 s, but only while someone can see it: this tab in front and the window focused.
const auto = ref(true)
const now = ref(Date.now())
let timer: ReturnType<typeof setInterval> | undefined
const visible = () => props.active && document.visibilityState === 'visible' && document.hasFocus()
onMounted(() => {
  void store.refreshServer()
  timer = setInterval(() => {
    now.value = Date.now()
    if (auto.value && visible() && now.value - (store.serverAt ?? 0) >= 5000) void store.refreshServer()
  }, 1000)
})
onBeforeUnmount(() => clearInterval(timer))
watch(() => props.active, (a) => { if (a) void store.refreshServer() })

const s = computed(() => store.stats)
const updated = computed(() => {
  if (!store.serverAt) return ''
  const secs = Math.max(0, Math.round((now.value - store.serverAt) / 1000))
  return secs < 2 ? 'updated just now' : `updated ${secs} s ago`
})
const plural = (n: number, one: string) => `${count(n)} ${one}${n === 1 ? '' : 's'}`
const tiles = computed(() => {
  const x = s.value
  if (!x) return []
  const up = x.uptime_secs ?? 0
  const lookups = (x.hits ?? 0) + (x.misses ?? 0)
  const used = x.memory_used ?? 0
  const max = x.memory_max ?? 0
  return [
    { label: 'Version', value: x.version, note: x.role === 'replica' ? `${x.mode} · replica` : x.mode },
    { label: 'Uptime', value: up >= 86400 ? plural(Math.floor(up / 86400), 'day') : up >= 3600 ? plural(Math.floor(up / 3600), 'hour') : plural(Math.floor(up / 60), 'minute'), note: `since ${new Date(Date.now() - up * 1000).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}` },
    { label: 'Memory', value: sizeText(used), note: max ? `of ${sizeText(max)} · ${Math.round((used / max) * 100)}%` : 'no limit set' },
    { label: 'Clients', value: count(x.clients ?? 0), note: `${count(x.blocked_clients ?? 0)} blocked` },
    { label: 'Commands', value: `${count(x.ops_per_sec ?? 0)} /s`, note: 'now' },
    { label: 'Hit rate', value: lookups ? `${(((x.hits ?? 0) / lookups) * 100).toFixed(1)}%` : '—', note: lookups ? `of ${plural(lookups, 'lookup')}` : 'no lookups yet' },
  ]
})
const when = (secs: number) => new Date(secs * 1000).toLocaleTimeString('en-GB', { hour12: false })
const took = (micros: number) => (micros >= 1000 ? `${count(Math.round(micros / 1000))} ms` : `${(micros / 1000).toFixed(1)} ms`)
const clearLog = () => store.consoleRun('SLOWLOG RESET', 'Clear the slow log?', () => store.refreshServer())
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col gap-4 overflow-auto p-4" :data-testid="tid('redis-server')">
    <div class="flex items-center gap-2">
      <h2 class="text-sm font-semibold">Server</h2>
      <span class="text-xs text-muted" aria-live="off">{{ updated }}</span>
      <Button variant="secondary" size="sm" class="ml-auto" :aria-pressed="auto" :class="auto ? 'shadow-[inset_0_-2px_0_var(--accent)]' : ''" @click="auto = !auto">Refresh every 5 s</Button>
      <IconButton label="Refresh now" :disabled="store.serverLoading" @click="store.refreshServer()"><RefreshCw /></IconButton>
    </div>

    <div v-if="store.serverError && !s" class="flex items-center gap-2.5 rounded-lg border border-border bg-surface px-3 py-2.5" role="alert">
      <CircleAlert class="size-4 shrink-0 text-danger" aria-hidden="true" /><span class="min-w-0 flex-1 text-sm">{{ store.serverError }}</span>
      <Button variant="secondary" size="sm" @click="store.refreshServer()">Try again</Button>
    </div>
    <div v-else-if="!s" class="grid grid-cols-6 gap-3" aria-busy="true" aria-label="Loading the server's details"><div v-for="i in 6" :key="i" class="h-[84px] animate-pulse rounded-[10px] bg-surface" /></div>

    <template v-else>
      <dl class="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
        <div v-for="t in tiles" :key="t.label" class="flex flex-col gap-0.5 rounded-[10px] border border-border bg-surface px-3.5 py-3">
          <dt class="text-2xs font-medium uppercase tracking-wider text-muted">{{ t.label }}</dt>
          <dd class="text-xl font-semibold leading-7 tabular-nums">{{ t.value }}</dd>
          <dd class="text-xs text-muted">{{ t.note }}</dd>
        </div>
      </dl>

      <div class="grid min-h-0 flex-1 grid-cols-1 gap-4 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
        <section class="flex min-h-0 flex-col overflow-hidden rounded-[10px] border border-border bg-surface" aria-labelledby="slow-title">
          <div class="flex h-9 shrink-0 items-center gap-2 border-b border-border px-3">
            <h3 id="slow-title" class="font-medium">Slow commands</h3><span class="text-xs text-muted">newest first</span>
            <Button v-if="!readOnly && store.slowlog.length" variant="ghost" size="sm" class="ml-auto" @click="clearLog">Clear log…</Button>
          </div>
          <div class="min-h-0 flex-1 overflow-auto">
            <table v-if="store.slowlog.length" class="w-full border-collapse text-left text-xs" aria-labelledby="slow-title">
              <thead><tr class="h-7 text-muted"><th class="px-2.5 font-medium">When</th><th class="px-2.5 font-medium">Took</th><th class="w-full px-2.5 font-medium">Command</th><th class="px-2.5 font-medium">Client</th></tr></thead>
              <tbody>
                <tr v-for="e in store.slowlog" :key="e.id ?? 0" class="h-[30px] border-t border-surface-raised">
                  <td class="whitespace-nowrap px-2.5 text-muted">{{ when(e.at_secs ?? 0) }}</td><td class="whitespace-nowrap px-2.5 font-mono">{{ took(e.micros ?? 0) }}</td>
                  <td class="max-w-0 truncate px-2.5 font-mono" :title="e.command">{{ e.command }}</td><td class="px-2.5 font-mono">{{ e.client || '—' }}</td>
                </tr>
              </tbody>
            </table>
            <p v-else class="p-3 text-xs text-muted">Nothing slow has been logged.</p>
          </div>
          <p class="shrink-0 border-t border-border px-3 py-2.5 text-xs text-muted">Read from the server's own slow log. Dalan never turns on live command tracing.</p>
        </section>

        <section class="flex min-h-0 flex-col overflow-hidden rounded-[10px] border border-border bg-surface" aria-label="Keys and nodes">
          <div class="flex h-9 shrink-0 items-center border-b border-border px-3"><h3 id="dbs-title" class="font-medium">Keys by database</h3></div>
          <table v-if="s.databases.length" class="w-full border-collapse text-left text-xs" aria-labelledby="dbs-title">
            <thead><tr class="h-7 text-muted"><th class="px-2.5 font-medium">Database</th><th class="px-2.5 font-medium">Keys</th><th class="px-2.5 font-medium">With expiry</th><th class="px-2.5 font-medium">Avg. time left</th></tr></thead>
            <tbody>
              <tr v-for="d in s.databases" :key="d.index" class="h-[30px] border-t border-surface-raised">
                <td class="px-2.5 text-muted">db{{ d.index }}</td><td class="px-2.5 font-mono">{{ count(d.keys ?? 0) }}</td><td class="px-2.5 font-mono">{{ count(d.expires ?? 0) }}</td>
                <td class="px-2.5 font-mono">{{ d.expires && d.avg_ttl_ms ? ttlText(d.avg_ttl_ms) : '—' }}</td>
              </tr>
            </tbody>
          </table>
          <p v-else class="p-3 text-xs text-muted">This server holds no keys.</p>

          <template v-if="store.nodes.length">
            <div class="flex h-9 shrink-0 items-center gap-2 border-y border-border px-3"><h3 id="nodes-title" class="font-medium">Nodes</h3><span class="text-xs text-muted">{{ s.mode === 'cluster' ? 'the tiles above are one node\'s; keys are the whole cluster\'s' : 'as the sentinels see them' }}</span></div>
            <div class="min-h-0 flex-1 overflow-auto">
              <table class="w-full border-collapse text-left text-xs" aria-labelledby="nodes-title">
                <thead><tr class="h-7 text-muted"><th class="px-2.5 font-medium">Role</th><th class="px-2.5 font-medium">Address</th><th class="px-2.5 font-medium">{{ s.mode === 'cluster' ? 'Slots' : 'Follows' }}</th><th class="px-2.5 font-medium">State</th></tr></thead>
                <tbody>
                  <tr v-for="n in store.nodes" :key="`${n.role}:${n.address}`" class="h-[30px] border-t border-surface-raised">
                    <td class="px-2.5 text-muted">{{ n.role }}</td><td class="whitespace-nowrap px-2.5 font-mono">{{ n.address }}</td><td class="px-2.5 font-mono">{{ n.detail }}</td>
                    <td :class="['px-2.5 font-mono', n.state === 'ok' ? 'text-success' : n.state === 'catching up' ? 'text-warning' : 'text-danger']">{{ n.state }}</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </template>
        </section>
      </div>
    </template>
  </div>
</template>
