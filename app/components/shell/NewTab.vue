<script setup lang="ts">
// S16: pick a host to connect in this tab. Recent hosts first; typing searches all of them.
import { toast } from 'vue-sonner'
import { LayoutGrid, Plug, Plus, Search, SquareTerminal } from '@lucide/vue'
import { Checkbox } from '@/components/ui/checkbox'
import type { HostSummary } from '~/types/bindings'
import { osLetter, osName } from '~/utils/os'
import { cn } from '@/lib/utils'
import { findQuickHost, parseQuickConnect } from '~/utils/quickConnect'

const emit = defineEmits<{ newHost: [] }>()
const hosts = useHostsStore()
const sessions = useSessionsStore()
const templates = useWorkspacesStore()
/** Hosts ticked to open together as a workspace (S16 -> S19). */
const checked = ref<string[]>([])
const toggle = (id: string) => { checked.value = checked.value.includes(id) ? checked.value.filter(x => x !== id) : [...checked.value, id] }
function openChecked() {
  const chosen = hosts.hosts.filter(h => checked.value.includes(h.id))
  if (chosen.length >= 2) sessions.openWorkspace(chosen)
}

const query = ref('')
const index = ref(0)
const input = ref<HTMLInputElement | null>(null)

const results = computed<HostSummary[]>(() => {
  const q = query.value.trim().toLowerCase()
  const all = [...hosts.hosts]
  if (!q) {
    // Hosts you have connected to, newest first; if there are none yet, show them all.
    // The servers you connected to lately (from the connection history), newest first; none yet: show them all.
    const byId = new Map(all.map(h => [h.id, h]))
    const recent = hosts.recent.map(r => byId.get(r.host_id)).filter((h): h is HostSummary => !!h)
    return (recent.length ? recent : all).slice(0, 12)
  }
  return all.filter(h => `${h.label} ${h.address} ${h.effective_username ?? ''} ${h.tags.join(' ')}`.toLowerCase().includes(q)).slice(0, 12)
})
watch(query, () => { index.value = 0 })

// `user@host[:port]` that matches no saved host: connect to it. It is saved first (a connection needs a host
// record in the vault), so it also shows up under Recent next time and can be deleted from Hosts.
const { label: shortcutLabel } = useShortcutLabel()
const quick = computed(() => (results.value.length ? null : parseQuickConnect(query.value)))
const quickLabel = computed(() => (quick.value ? `${quick.value.username ? `${quick.value.username}@` : ''}${quick.value.address}${quick.value.port ? `:${quick.value.port}` : ''}` : ''))
async function connectQuick() {
  const q = quick.value
  if (!q) return
  const existing = findQuickHost(hosts.hosts, q)
  if (existing) return void sessions.open(existing)
  const err = await hosts.save({ id: null, label: q.address, address: q.address, port: q.port, username: q.username, password: null, clear_password: false, group_id: null, key_id: null, identity_id: null, startup_snippet_id: null, jump_mode: 'inherit', jump_host_ids: [], tags: [] })
  if (err) return void toast.error(err)
  const saved = findQuickHost(hosts.hosts, q)
  if (saved) sessions.open(saved)
}

function ago(ms: number | null | undefined) {
  if (!ms) return 'Not connected yet'
  const mins = Math.round((Date.now() - ms) / 60000)
  if (mins < 1) return 'just now'
  if (mins < 60) return `${mins} min ago`
  if (mins < 60 * 24) return `${Math.round(mins / 60)} h ago`
  return new Date(ms).toLocaleDateString([], { month: 'short', day: 'numeric' })
}

function onKey(e: KeyboardEvent) {
  if (e.key === 'ArrowDown') { e.preventDefault(); index.value = Math.min(index.value + 1, results.value.length - 1) }
  else if (e.key === 'ArrowUp') { e.preventDefault(); index.value = Math.max(index.value - 1, 0) }
  else if (e.key === 'Enter' && checked.value.length >= 2) { e.preventDefault(); openChecked() }
  else if (e.key === 'Enter' && quick.value) { e.preventDefault(); void connectQuick() }
  else if (e.key === 'Enter' && results.value[index.value]) { e.preventDefault(); sessions.open(results.value[index.value]!) }
  else if (e.key === ' ' && e.ctrlKey && results.value[index.value]) { e.preventDefault(); toggle(results.value[index.value]!.id) }
}
onMounted(() => input.value?.focus())
defineExpose({ focus: () => input.value?.focus() })
</script>

<template>
  <main class="flex size-full justify-center overflow-auto bg-bg px-6 py-16">
    <div class="flex w-[560px] max-w-full flex-col gap-5">
      <label class="relative block">
        <Search class="pointer-events-none absolute left-3 top-3 size-4 text-muted" aria-hidden="true" />
        <input
          ref="input"
          v-model="query"
          aria-label="Search hosts"
          placeholder="Type a host, address or tag"
          class="h-11 w-full rounded-lg border border-border-input bg-bg pl-10 pr-3 text-base outline-none placeholder:text-muted focus:border-accent focus:ring-2 focus:ring-accent/40"
          @keydown="onKey"
        >
      </label>
      <div class="flex flex-wrap gap-2">
        <Button variant="secondary" @click="emit('newHost')"><Plus />New host</Button>
        <Button variant="secondary" @click="sessions.openLocal()"><SquareTerminal />Local terminal</Button>
      </div>
      <section class="flex flex-col gap-1" aria-label="Recent">
        <div class="flex items-center px-3 pb-1.5">
          <h2 class="text-xs font-medium text-muted">{{ query ? 'Matches' : 'Recent' }}</h2>
          <Button v-if="checked.length >= 2" size="sm" class="ml-auto" @click="openChecked"><LayoutGrid />Open {{ checked.length }} as workspace</Button>
        </div>
        <button v-if="quick" type="button" class="flex h-11 items-center gap-3 rounded-md bg-accent/10 px-3 text-left hover:bg-surface-hover" @click="connectQuick">
          <Plug class="size-4 text-accent" aria-hidden="true" />
          <span class="truncate">Connect to <span class="font-mono">{{ quickLabel }}</span></span>
          <span class="ml-auto text-xs text-muted">Saved to Hosts</span>
        </button>
        <p v-else-if="!results.length" class="px-3 py-4 text-muted">{{ hosts.hosts.length ? 'No matches.' : 'No hosts yet. Add one to connect.' }}</p>
        <button
          v-for="(h, i) in results"
          :key="h.id"
          type="button"
          :class="cn('group flex h-11 items-center gap-3 rounded-md px-3 text-left hover:bg-surface-hover', (i === index || checked.includes(h.id)) && 'bg-accent/10')"
          @mousemove="index = i"
          @click="sessions.open(h)"
        >
          <Checkbox :model-value="checked.includes(h.id)" :aria-label="`Select ${h.label} for a workspace`" :class="checked.length ? '' : 'opacity-0 group-hover:opacity-100 focus-visible:opacity-100'" @click.stop @update:model-value="toggle(h.id)" />
          <span class="grid size-7 shrink-0 place-items-center rounded-md bg-surface-hover text-2xs font-semibold" :title="osName(h.os_hint)">{{ osLetter(h.os_hint) ?? 'S' }}</span>
          <span class="flex min-w-0 flex-col">
            <span class="truncate font-medium">{{ h.label }}</span>
            <span class="truncate font-mono text-xs text-muted"><template v-if="h.effective_username">{{ h.effective_username }}@</template>{{ h.address }}<template v-if="h.effective_port !== 22">:{{ h.effective_port }}</template></span>
          </span>
          <span class="ml-auto text-xs text-muted">{{ ago(h.last_connected_at) }}</span>
        </button>
      </section>
      <section v-if="templates.templates.length" class="flex flex-col gap-1" aria-label="Workspace templates">
        <h2 class="px-3 pb-1.5 text-xs font-medium text-muted">Workspaces</h2>
        <button v-for="t in templates.templates" :key="t.id" type="button" class="flex h-11 items-center gap-3 rounded-md px-3 text-left hover:bg-surface-hover" @click="templates.open(t)">
          <LayoutGrid class="size-4 text-accent" aria-hidden="true" />
          <span class="font-medium">{{ t.name }}</span>
          <span class="ml-auto text-xs text-muted">{{ JSON.parse(t.panes).length }} panes</span>
        </button>
      </section>
      <p class="mt-2 flex flex-wrap gap-4 text-xs text-muted">
        <span><Kbd>↵</Kbd> connect</span><span><Kbd>↑↓</Kbd> choose</span><span><Kbd>Ctrl Space</Kbd> select for workspace</span><span><Kbd>{{ shortcutLabel('palette') }}</Kbd> everything</span>
      </p>
    </div>
  </main>
</template>
