<script setup lang="ts">
// S16: pick a host to connect in this tab. Recent hosts first; typing searches all of them.
import { LayoutGrid, Plus, Search, SquareTerminal } from '@lucide/vue'
import { Checkbox } from '@/components/ui/checkbox'
import type { HostSummary } from '~/types/bindings'
import { cn } from '@/lib/utils'

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
    const recent = all.filter(h => h.last_connected_at).sort((a, b) => (b.last_connected_at ?? 0) - (a.last_connected_at ?? 0))
    return (recent.length ? recent : all).slice(0, 12)
  }
  return all.filter(h => `${h.label} ${h.address} ${h.effective_username ?? ''} ${h.tags.join(' ')}`.toLowerCase().includes(q)).slice(0, 12)
})
watch(query, () => { index.value = 0 })

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
          class="h-11 w-full rounded-lg border border-border-input bg-bg pl-10 pr-3 text-[15px] outline-none placeholder:text-muted focus:border-accent focus:ring-2 focus:ring-accent/40"
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
        <p v-if="!results.length" class="px-3 py-4 text-muted">{{ hosts.hosts.length ? 'No matches.' : 'No hosts yet. Add one to connect.' }}</p>
        <button
          v-for="(h, i) in results"
          :key="h.id"
          type="button"
          :class="cn('group flex h-11 items-center gap-3 rounded-md px-3 text-left hover:bg-surface-hover', (i === index || checked.includes(h.id)) && 'bg-accent/10')"
          @mousemove="index = i"
          @click="sessions.open(h)"
        >
          <Checkbox :model-value="checked.includes(h.id)" :aria-label="`Select ${h.label} for a workspace`" :class="checked.length ? '' : 'opacity-0 group-hover:opacity-100 focus-visible:opacity-100'" @click.stop @update:model-value="toggle(h.id)" />
          <span class="grid size-7 shrink-0 place-items-center rounded-md bg-surface-hover text-2xs font-semibold">S</span>
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
        <span><Kbd>↵</Kbd> connect</span><span><Kbd>↑↓</Kbd> choose</span><span><Kbd>Ctrl Space</Kbd> select for workspace</span><span><Kbd>⌘K</Kbd> everything</span>
      </p>
    </div>
  </main>
</template>
