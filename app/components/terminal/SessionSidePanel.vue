<script setup lang="ts">
// S20 (snippets tab): run a saved script in this session. Clicking a row types it into the shell.
import { Play, X } from '@lucide/vue'
import { toast } from 'vue-sonner'

const snippets = useSnippetsStore()
const sessions = useSessionsStore()
const query = ref('')

const matches = computed(() => {
  const q = query.value.trim().toLowerCase()
  return snippets.snippets.filter(s => !q || `${s.label} ${s.script}`.toLowerCase().includes(q))
})
function run(id: string) {
  const s = snippets.snippets.find(x => x.id === id)
  if (s && snippets.runInActive(s)) toast(`Ran ${s.label}.`)
}
</script>

<template>
  <aside aria-label="Side panel" class="flex h-full w-[300px] shrink-0 flex-col border-l border-border bg-surface">
    <div class="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
      <h2 class="text-sm font-medium">Snippets</h2>
      <IconButton label="Close side panel" shortcut="⌘." size="xs" class="ml-auto" @click="sessions.panelOpen = false"><X /></IconButton>
    </div>
    <div class="p-2"><SearchInput v-model="query" class="w-full" placeholder="Find a snippet" :result-count="matches.length" /></div>
    <div class="flex-1 overflow-auto px-2 pb-2">
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
  </aside>
</template>
