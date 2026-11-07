<script setup lang="ts">
// S17: jump to a host, a tab or an action. Built on the shadcn Command (Reka listbox) dialog.
import { ScrollText, ArrowDownUp, Code2, LayoutGrid, KeyRound, Waypoints, Lock, Plus, Server, Settings, ShieldCheck, SquareTerminal, Vault } from '@lucide/vue'
import { CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandSeparator, CommandShortcut } from '@/components/ui/command'
import type { HostSummary } from '~/types/bindings'
const { label: shortcutLabel, fixed: shortcutFixed } = useShortcutLabel()

const open = defineModel<boolean>('open', { required: true })
const emit = defineEmits<{ action: [name: 'new-host' | 'new-tab' | 'local' | 'sftp' | 'keychain' | 'forwards' | 'snippets' | 'logs' | 'known-hosts' | 'settings' | 'lock' | 'vault'] }>()
const hosts = useHostsStore()
const sessions = useSessionsStore()
const templates = useWorkspacesStore()

const run = (fn: () => void) => {
  open.value = false
  nextTick(fn) // let the dialog close first, so focus returns to the right place
}
const connect = (h: HostSummary) => run(() => sessions.open(h))
const gotoTab = (key: string) => run(() => { sessions.activeKey = key })
const act = (name: Parameters<typeof emit>[1]) => run(() => emit('action', name))
const openTabs = computed(() => sessions.sessions.filter(s => s.kind === 'session'))

// A vault can hold thousands of hosts, and mounting one list item per host made every keystroke take ~300 ms with 2,000
// (test-plan 9.4). So only the best few matches are mounted: we filter by the typed text here, and the list widget
// then filters those again the same way. Hosts keep the order they have in the store.
const HOST_LIMIT = 50
const typed = ref('')
watch(open, (o) => { if (o) typed.value = '' })
const hostValue = (h: HostSummary) => `host ${h.label} ${h.address} ${h.tags.join(' ')}`
const matchingHosts = computed(() => {
  const q = typed.value.trim().toLowerCase()
  return q ? hosts.hosts.filter(h => hostValue(h).toLowerCase().includes(q)) : hosts.hosts
})
const shownHosts = computed(() => matchingHosts.value.slice(0, HOST_LIMIT))
</script>

<template>
  <CommandDialog v-model:open="open" title="Command palette" description="Jump to a host, a tab or an action.">
    <CommandInput placeholder="Search hosts, tabs and actions" @input="typed = ($event.target as HTMLInputElement).value" />
    <CommandList aria-label="Results">
      <CommandEmpty>No matches. Press {{ shortcutLabel('new-host') }} to add a host.</CommandEmpty>

      <CommandGroup v-if="openTabs.length" heading="Tabs">
        <CommandItem v-for="s in openTabs" :key="s.key" :value="`tab ${s.label} ${s.address}`" @select="gotoTab(s.key)">
          <SquareTerminal />{{ s.label }}<span class="ml-2 font-mono text-xs text-muted">{{ s.address }}</span>
        </CommandItem>
      </CommandGroup>

      <CommandGroup v-if="hosts.hosts.length" heading="Hosts">
        <CommandItem v-for="h in shownHosts" :key="h.id" :value="hostValue(h)" @select="connect(h)">
          <Server />{{ h.label }}<span class="ml-2 font-mono text-xs text-muted">{{ h.effective_username ? `${h.effective_username}@` : '' }}{{ h.address }}</span>
        </CommandItem>
        <p v-if="matchingHosts.length > HOST_LIMIT" class="px-2 py-1.5 text-xs text-muted">Showing {{ HOST_LIMIT }} of {{ matchingHosts.length }} hosts. Keep typing to narrow it down.</p>
      </CommandGroup>

      <CommandGroup v-if="templates.templates.length" heading="Workspaces">
        <CommandItem v-for="t in templates.templates" :key="t.id" :value="`workspace ${t.name}`" @select="run(() => templates.open(t))"><LayoutGrid />{{ t.name }}</CommandItem>
      </CommandGroup>

      <CommandSeparator role="none" aria-hidden="true" />
      <CommandGroup heading="Actions">
        <CommandItem value="action new host add" @select="act('new-host')"><Plus />New host<CommandShortcut>{{ shortcutLabel('new-host') }}</CommandShortcut></CommandItem>
        <CommandItem value="action new tab" @select="act('new-tab')"><Plus />New tab<CommandShortcut>{{ shortcutLabel('new-tab') }}</CommandShortcut></CommandItem>
        <CommandItem value="action local terminal shell this computer" @select="act('local')"><SquareTerminal />Open local terminal</CommandItem>
        <CommandItem value="action vault hosts" @select="act('vault')"><Vault />Go to Hosts<CommandShortcut>{{ shortcutFixed('Mod+1') }}</CommandShortcut></CommandItem>
        <CommandItem value="action sftp files transfer" @select="act('sftp')"><ArrowDownUp />Go to SFTP</CommandItem>
        <CommandItem value="action keychain keys identities" @select="act('keychain')"><KeyRound />Go to Keychain</CommandItem>
        <CommandItem value="action port forwarding tunnels" @select="act('forwards')"><Waypoints />Go to Port forwarding</CommandItem>
        <CommandItem value="action snippets scripts" @select="act('snippets')"><Code2 />Go to Snippets</CommandItem>
        <CommandItem value="action logs session history" @select="act('logs')"><ScrollText />Go to Logs</CommandItem>
        <CommandItem value="action known hosts fingerprints" @select="act('known-hosts')"><ShieldCheck />Go to Known hosts</CommandItem>
        <CommandItem value="action settings preferences" @select="act('settings')"><Settings />Settings<CommandShortcut>{{ shortcutLabel('settings') }}</CommandShortcut></CommandItem>
        <CommandItem value="action lock vault" @select="act('lock')"><Lock />Lock Dalan</CommandItem>
      </CommandGroup>
    </CommandList>
  </CommandDialog>
</template>
