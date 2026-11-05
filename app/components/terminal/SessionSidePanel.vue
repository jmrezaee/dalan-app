<script setup lang="ts">
// S20: snippets and command history for the current session. A snippet or command row types into the shell.
import { CornerDownLeft, Copy, Play, Sparkles, Trash2, X } from '@lucide/vue'
import { toast } from 'vue-sonner'
import type { HistoryEntry } from '~/types/bindings'
import { cn } from '@/lib/utils'

const snippets = useSnippetsStore()
const sessions = useSessionsStore()
const settings = useSettingsStore()
const api = useTauri()
const tab = ref<'snippets' | 'history' | 'ai'>('snippets')
const query = ref('')

const matches = computed(() => {
  const q = query.value.trim().toLowerCase()
  return snippets.snippets.filter(s => !q || `${s.label} ${s.script}`.toLowerCase().includes(q))
})
function run(id: string) {
  const s = snippets.snippets.find(x => x.id === id)
  if (s && snippets.runInActive(s)) toast(`Ran ${s.label}.`)
}

// ---- History (per server; the local terminal has its own).
const history = ref<HistoryEntry[]>([])
const hostId = computed(() => sessions.activeSession?.hostId || null)
async function loadHistory() {
  try {
    history.value = await api.historyList(hostId.value, tab.value === 'history' ? query.value.trim() || null : null, 200)
  }
  catch {
    history.value = []
  }
}
watch([tab, hostId, () => sessions.panelOpen], () => { if (tab.value === 'history' && sessions.panelOpen) loadHistory() }, { immediate: true })
watch(query, () => { if (tab.value === 'history') loadHistory() })
/** Types the command; with `enter` it runs, without it you can edit first. */
function use(c: HistoryEntry, enter: boolean) {
  const s = sessions.activeSession
  if (!s) return
  sessions.input(s, new TextEncoder().encode(enter ? `${c.command}\n` : c.command))
}
// ---- Ask AI: one suggested command, typed into the shell for you to check and run yourself.
const ask = reactive({ text: '', busy: false, error: '', suggestion: null as { command: string, is_question: boolean } | null })
async function suggest() {
  if (ask.busy || !ask.text.trim()) return
  ask.busy = true
  ask.error = ''
  ask.suggestion = null
  try {
    ask.suggestion = await api.aiSuggest(ask.text, hostId.value)
  }
  catch (e) {
    ask.error = e instanceof Error ? e.message : 'Something went wrong. Try again.'
  }
  ask.busy = false
}
function insertSuggestion() {
  const s = sessions.activeSession
  if (s && ask.suggestion && !ask.suggestion.is_question) sessions.input(s, new TextEncoder().encode(ask.suggestion.command))
}
async function copySuggestion() {
  if (ask.suggestion) await navigator.clipboard.writeText(ask.suggestion.command).then(() => toast('Copied.')).catch(() => {})
}
async function clearHistory() {
  try {
    await api.historyClear(hostId.value, false)
    toast('History cleared.')
  }
  catch {
    toast.error('Something went wrong. Try again.')
  }
  await loadHistory()
}
</script>

<template>
  <aside aria-label="Side panel" class="flex h-full w-[300px] shrink-0 flex-col border-l border-border bg-surface">
    <div class="flex h-10 shrink-0 items-center gap-1 border-b border-border px-2">
      <div role="tablist" aria-label="Side panel" class="flex gap-1">
        <button
          v-for="t in [{ v: 'snippets', l: 'Snippets' }, { v: 'history', l: 'History' }, { v: 'ai', l: 'Ask AI' }] as const"
          :key="t.v"
          type="button"
          role="tab"
          :aria-selected="tab === t.v"
          :class="cn('h-7 rounded-md px-2.5 text-sm', tab === t.v ? 'bg-surface-hover font-medium' : 'text-muted hover:bg-surface-hover')"
          @click="tab = t.v; query = ''"
        >
          {{ t.l }}
        </button>
      </div>
      <IconButton label="Close side panel" shortcut="⌘." size="xs" class="ml-auto" @click="sessions.panelOpen = false"><X /></IconButton>
    </div>
    <div v-if="tab !== 'ai'" class="p-2"><SearchInput v-model="query" class="w-full" :placeholder="tab === 'snippets' ? 'Find a snippet' : 'Find a command'" :result-count="tab === 'snippets' ? matches.length : history.length" /></div>

    <div v-if="tab === 'snippets'" class="flex-1 overflow-auto px-2 pb-2">
      <p v-if="!snippets.snippets.length" class="px-2 py-4 text-sm text-muted">No snippets yet. Create some under Snippets in the vault.</p>
      <p v-else-if="!matches.length" class="px-2 py-4 text-sm text-muted">No snippets match.</p>
      <button
        v-for="s in matches"
        :key="s.id"
        type="button"
        class="group flex w-full items-center gap-2 rounded-md px-2 py-2 text-left hover:bg-surface-hover"
        @click="run(s.id)"
      >
        <span class="flex min-w-0 flex-1 flex-col">
          <span class="truncate text-sm">{{ s.label }}</span>
          <span class="truncate font-mono text-2xs text-muted">{{ s.script.split('\n')[0] }}</span>
        </span>
        <Play class="size-3.5 shrink-0 text-muted group-hover:text-accent" aria-label="Run" />
      </button>
    </div>

    <div v-else-if="tab === 'ai'" class="flex min-h-0 flex-1 flex-col gap-3 overflow-auto p-3">
      <label class="flex flex-col gap-1.5 text-xs text-muted">Describe what you want to do
        <textarea v-model="ask.text" autocapitalize="off" autocorrect="off" rows="3" maxlength="1000" placeholder="show the 10 biggest files in /var/log" class="resize-none rounded-md border border-border-input bg-bg p-2 text-sm text-text outline-none focus:border-accent" @keydown.enter.exact.prevent="suggest" />
      </label>
      <Button :loading="ask.busy" :disabled="!ask.text.trim()" @click="suggest"><Sparkles />Suggest a command</Button>
      <p v-if="ask.error" role="alert" class="text-xs text-danger">{{ ask.error }} <span v-if="ask.error.includes('API key')">Add it under Settings, then AI.</span></p>
      <div v-if="ask.suggestion" class="flex flex-col gap-2 rounded-md border border-border bg-surface-raised p-2.5">
        <p v-if="ask.suggestion.is_question" class="text-sm">{{ ask.suggestion.command }}</p>
        <template v-else>
          <code class="whitespace-pre-wrap break-all font-mono text-xs">{{ ask.suggestion.command }}</code>
          <div class="flex gap-2">
            <Button size="sm" @click="insertSuggestion"><CornerDownLeft />Insert</Button>
            <Button size="sm" variant="secondary" @click="copySuggestion"><Copy />Copy</Button>
          </div>
          <p class="text-2xs text-muted">Check it before you press Enter. AI can be wrong.</p>
        </template>
      </div>
      <p class="text-2xs text-muted">Sends your text and the server's OS name to your AI provider. Nothing from the terminal.</p>
    </div>

    <div v-else class="flex min-h-0 flex-1 flex-col">
      <div class="flex-1 overflow-auto px-2 pb-2">
        <p v-if="!settings.historyEnabled" class="px-2 py-4 text-sm text-muted">Command history is off. Turn it on in Settings, under Logs.</p>
        <p v-else-if="!history.length" class="px-2 py-4 text-sm text-muted">{{ query ? 'No commands match.' : 'Commands you type here will show up.' }}</p>
        <div v-for="c in history" :key="c.id" class="group flex items-center gap-1 rounded-md px-2 py-1.5 hover:bg-surface-hover">
          <button type="button" class="min-w-0 flex-1 truncate text-left font-mono text-xs" :title="c.command" @click="use(c, false)">{{ c.command }}</button>
          <IconButton label="Run command" size="xs" class="opacity-0 group-hover:opacity-100 focus-visible:opacity-100" @click="use(c, true)"><CornerDownLeft /></IconButton>
        </div>
      </div>
      <div v-if="history.length" class="border-t border-border p-2">
        <Button variant="ghost" size="sm" class="w-full justify-start text-muted" @click="clearHistory"><Trash2 />Clear history for this server</Button>
      </div>
    </div>
  </aside>
</template>
