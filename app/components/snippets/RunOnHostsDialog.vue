<script setup lang="ts">
// S13: pick the hosts to run a snippet on. Opens a workspace with one pane per host and types the script in each.
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import type { SnippetSummary } from '~/types/bindings'
import { MAX_PANES } from '~/stores/sessions'

const open = defineModel<boolean>('open', { required: true })
const props = defineProps<{ snippet: SnippetSummary | null }>()
const hosts = useHostsStore()
const sessions = useSessionsStore()
const snippets = useSnippetsStore()

const picked = ref<string[]>([])
const query = ref('')
const remember = ref(true)

watch(open, (o) => {
  if (!o) return
  picked.value = (props.snippet?.target_host_ids ?? []).filter(id => hosts.hosts.some(h => h.id === id))
  query.value = ''
  remember.value = true
})

const groups = computed(() => {
  const q = query.value.trim().toLowerCase()
  const match = (h: (typeof hosts.hosts)[number]) => !q || `${h.label} ${h.address}`.toLowerCase().includes(q)
  const out = hosts.groups.map(g => ({ id: g.id, name: g.label, hosts: hosts.hosts.filter(h => h.group_id === g.id && match(h)) }))
  out.push({ id: '', name: 'Not in a group', hosts: hosts.hosts.filter(h => !h.group_id && match(h)) })
  return out.filter(g => g.hosts.length)
})
const toggle = (id: string) => { picked.value = picked.value.includes(id) ? picked.value.filter(x => x !== id) : [...picked.value, id] }
const toggleGroup = (ids: string[]) => {
  const all = ids.every(id => picked.value.includes(id))
  picked.value = all ? picked.value.filter(id => !ids.includes(id)) : [...new Set([...picked.value, ...ids])]
}
const tooMany = computed(() => picked.value.length > MAX_PANES)
const note = (h: { effective_has_password: boolean, effective_key_id?: string | null }) => (!h.effective_has_password && !h.effective_key_id ? 'asks for password' : '')

async function run() {
  const s = props.snippet
  if (!s || !picked.value.length || tooMany.value) return
  const chosen = hosts.hosts.filter(h => picked.value.includes(h.id))
  if (remember.value) await snippets.save({ id: s.id, label: s.label, script: s.script, package_id: s.package_id, target_host_ids: picked.value })
  open.value = false
  sessions.openWorkspace(chosen, { label: s.label, autorun: s.script })
}
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="gap-0 p-0 sm:max-w-[560px]">
      <DialogHeader class="gap-2.5 p-5 pb-3">
        <DialogTitle>Run "{{ snippet?.label }}" on…</DialogTitle>
        <DialogDescription class="sr-only">Choose the servers to run this snippet on.</DialogDescription>
        <pre class="rounded-md border border-border bg-terminal-bg px-3 py-2.5 font-mono text-[13px] leading-[18px] text-terminal-fg">{{ snippet?.script }}</pre>
        <Input v-model="query" aria-label="Filter hosts" placeholder="Filter hosts" autocomplete="off" />
      </DialogHeader>
      <div class="max-h-[360px] overflow-auto border-y border-border px-2.5">
        <p v-if="!groups.length" class="px-2.5 py-4 text-sm text-muted">No hosts match.</p>
        <div v-for="g in groups" :key="g.id">
          <div class="flex items-center px-2.5 pb-1 pt-2 text-2xs font-medium uppercase tracking-wider text-muted">
            {{ g.name }}
            <button type="button" class="ml-auto text-xs normal-case tracking-normal text-accent hover:underline" @click="toggleGroup(g.hosts.map(h => h.id))">Select all</button>
          </div>
          <label v-for="h in g.hosts" :key="h.id" class="flex h-9 cursor-pointer items-center gap-2.5 rounded-md px-2.5 hover:bg-surface-hover">
            <Checkbox :model-value="picked.includes(h.id)" @update:model-value="toggle(h.id)" />
            <span class="font-medium">{{ h.label }}</span><span class="font-mono text-xs text-muted">{{ h.address }}</span>
            <span class="ml-auto text-xs text-warning">{{ note(h) }}</span>
          </label>
        </div>
      </div>
      <div class="flex flex-wrap items-center gap-2 px-5 py-3.5">
        <label class="mr-auto flex items-center gap-2 text-sm text-muted"><Checkbox v-model="remember" />Remember these hosts for this snippet</label>
        <Button variant="secondary" @click="open = false">Cancel</Button>
        <Button :disabled="!picked.length || tooMany" @click="run">Run on {{ picked.length || '…' }} {{ picked.length === 1 ? 'host' : 'hosts' }}</Button>
      </div>
      <p class="px-5 pb-3.5 text-xs text-muted">
        <template v-if="tooMany">A workspace holds {{ MAX_PANES }} panes. Choose fewer hosts.</template>
        <template v-else>Opens a workspace with one pane per host. Hosts without saved credentials ask for them in their own pane.</template>
      </p>
    </DialogContent>
  </Dialog>
</template>
