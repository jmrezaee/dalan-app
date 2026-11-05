<script setup lang="ts">
// S16: pick a host to connect in this tab. Recent hosts first; typing searches all of them.
import { Plus, Search } from '@lucide/vue'
import type { HostSummary } from '~/types/bindings'
import { cn } from '@/lib/utils'

const emit = defineEmits<{ newHost: [] }>()
const hosts = useHostsStore()
const sessions = useSessionsStore()

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
  else if (e.key === 'Enter' && results.value[index.value]) { e.preventDefault(); sessions.open(results.value[index.value]!) }
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
      </div>
      <section class="flex flex-col gap-1" aria-label="Recent">
        <h2 class="px-3 pb-1.5 text-xs font-medium text-muted">{{ query ? 'Matches' : 'Recent' }}</h2>
        <p v-if="!results.length" class="px-3 py-4 text-muted">{{ hosts.hosts.length ? 'No matches.' : 'No hosts yet. Add one to connect.' }}</p>
        <button
          v-for="(h, i) in results"
          :key="h.id"
          type="button"
          :class="cn('flex h-11 items-center gap-3 rounded-md px-3 text-left hover:bg-surface-hover', i === index && 'bg-accent/10')"
          @mousemove="index = i"
          @click="sessions.open(h)"
        >
          <span class="grid size-7 shrink-0 place-items-center rounded-md bg-surface-hover text-2xs font-semibold">S</span>
          <span class="flex min-w-0 flex-col">
            <span class="truncate font-medium">{{ h.label }}</span>
            <span class="truncate font-mono text-xs text-muted"><template v-if="h.effective_username">{{ h.effective_username }}@</template>{{ h.address }}<template v-if="h.effective_port !== 22">:{{ h.effective_port }}</template></span>
          </span>
          <span class="ml-auto text-xs text-muted">{{ ago(h.last_connected_at) }}</span>
        </button>
      </section>
      <p class="mt-2 flex flex-wrap gap-4 text-xs text-muted">
        <span><Kbd>↵</Kbd> connect</span><span><Kbd>↑↓</Kbd> choose</span><span><Kbd>⌘K</Kbd> everything</span>
      </p>
    </div>
  </main>
</template>
