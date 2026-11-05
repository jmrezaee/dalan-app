<script setup lang="ts">
// S17: jump to a host, a tab or an action. Built on the shadcn Command (Reka listbox) dialog.
import { Code2, KeyRound, Waypoints, Lock, Plus, Server, Settings, ShieldCheck, SquareTerminal, Vault } from '@lucide/vue'
import { CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandSeparator, CommandShortcut } from '@/components/ui/command'
import type { HostSummary } from '~/types/bindings'

const open = defineModel<boolean>('open', { required: true })
const emit = defineEmits<{ action: [name: 'new-host' | 'new-tab' | 'keychain' | 'forwards' | 'snippets' | 'known-hosts' | 'settings' | 'lock' | 'vault'] }>()
const hosts = useHostsStore()
const sessions = useSessionsStore()

const run = (fn: () => void) => {
  open.value = false
  nextTick(fn) // let the dialog close first, so focus returns to the right place
}
const connect = (h: HostSummary) => run(() => sessions.open(h))
const gotoTab = (key: string) => run(() => { sessions.activeKey = key })
const act = (name: Parameters<typeof emit>[1]) => run(() => emit('action', name))
const openTabs = computed(() => sessions.sessions.filter(s => s.kind === 'session'))
</script>

<template>
  <CommandDialog v-model:open="open" title="Command palette" description="Jump to a host, a tab or an action.">
    <CommandInput placeholder="Search hosts, tabs and actions" />
    <CommandList>
      <CommandEmpty>No matches. Press ⌘N to add a host.</CommandEmpty>

      <CommandGroup v-if="openTabs.length" heading="Tabs">
        <CommandItem v-for="s in openTabs" :key="s.key" :value="`tab ${s.label} ${s.address}`" @select="gotoTab(s.key)">
          <SquareTerminal />{{ s.label }}<span class="ml-2 font-mono text-xs text-muted">{{ s.address }}</span>
        </CommandItem>
      </CommandGroup>

      <CommandGroup v-if="hosts.hosts.length" heading="Hosts">
        <CommandItem v-for="h in hosts.hosts" :key="h.id" :value="`host ${h.label} ${h.address} ${h.tags.join(' ')}`" @select="connect(h)">
          <Server />{{ h.label }}<span class="ml-2 font-mono text-xs text-muted">{{ h.effective_username ? `${h.effective_username}@` : '' }}{{ h.address }}</span>
        </CommandItem>
      </CommandGroup>

      <CommandSeparator />
      <CommandGroup heading="Actions">
        <CommandItem value="action new host add" @select="act('new-host')"><Plus />New host<CommandShortcut>⌘N</CommandShortcut></CommandItem>
        <CommandItem value="action new tab" @select="act('new-tab')"><Plus />New tab<CommandShortcut>⌘T</CommandShortcut></CommandItem>
        <CommandItem value="action vault hosts" @select="act('vault')"><Vault />Go to Hosts<CommandShortcut>⌘1</CommandShortcut></CommandItem>
        <CommandItem value="action keychain keys identities" @select="act('keychain')"><KeyRound />Go to Keychain</CommandItem>
        <CommandItem value="action port forwarding tunnels" @select="act('forwards')"><Waypoints />Go to Port forwarding</CommandItem>
        <CommandItem value="action snippets scripts" @select="act('snippets')"><Code2 />Go to Snippets</CommandItem>
        <CommandItem value="action known hosts fingerprints" @select="act('known-hosts')"><ShieldCheck />Go to Known hosts</CommandItem>
        <CommandItem value="action settings preferences" @select="act('settings')"><Settings />Settings<CommandShortcut>⌘,</CommandShortcut></CommandItem>
        <CommandItem value="action lock vault" @select="act('lock')"><Lock />Lock Dalan</CommandItem>
      </CommandGroup>
    </CommandList>
  </CommandDialog>
</template>
