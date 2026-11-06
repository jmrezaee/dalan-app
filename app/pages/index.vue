<script setup lang="ts">
// The app shell: tab bar, the Vault sections (S02 Hosts and the rest), terminals kept mounted, and the global shortcuts.
import { ScrollText, ArrowDownUp, Code2, Copy, LayoutGrid, KeyRound, Lock, Server, Settings, ShieldCheck, Trash2, Vault, Waypoints } from '@lucide/vue'
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger } from '@/components/ui/context-menu'
import { buttonVariants } from '@/components/ui/button'
import type { Session } from '~/stores/sessions'
import type { MenuItem } from '@/components/common/ItemMenuList.vue'

const vault = useVaultStore()
const hosts = useHostsStore()
const sessions = useSessionsStore()
const keychain = useKeychainStore()
const snippetsStore = useSnippetsStore()
const logsStore = useLogsStore()
const forwardsStore = useForwardsStore()
const sftpStore = useSftpStore()
const workspaces = useWorkspacesStore()
const workspaceView = ref<{ askSave: () => void }[] | null>(null)
const settings = useSettingsStore()
const section = ref<'hosts' | 'keychain' | 'forwards' | 'snippets' | 'logs' | 'known-hosts' | 'settings'>('hosts')
const paletteOpen = ref(false)
const knownHosts = ref<{ refresh: () => void } | null>(null)
onMounted(() => {
  hosts.refresh()
  keychain.refresh()
  snippetsStore.refresh()
  forwardsStore.refresh()
  workspaces.refresh()
  forwardsStore.watch()
  settings.load()
})
onBeforeUnmount(() => {
  hosts.clear()
  keychain.clear()
  snippetsStore.clear()
  forwardsStore.clear()
  sftpStore.clear()
  workspaces.clear()
  logsStore.clear()
  sessions.clear()
})

const nav = computed(() => [
  { id: 'hosts', label: 'Hosts', icon: Server, count: hosts.hosts.length },
  { id: 'keychain', label: 'Keychain', icon: KeyRound, count: keychain.keys.length + keychain.identities.length },
  { id: 'forwards', label: 'Port forwarding', icon: Waypoints, count: forwardsStore.rules.length },
  { id: 'snippets', label: 'Snippets', icon: Code2, count: snippetsStore.snippets.length },
  { id: 'logs', label: 'Logs', icon: ScrollText },
  { id: 'known-hosts', label: 'Known hosts', icon: ShieldCheck },
])

function editHost(hostId: string) {
  const h = hosts.hosts.find(x => x.id === hostId)
  if (!h) return
  sessions.activeKey = null
  section.value = 'hosts'
  hosts.editing = h
}

// A tab's × while it is connected would end a live shell: ask first (only then).
const closing = ref<Session | null>(null)
const closeOpen = ref(false)
function requestClose(s: Session) {
  const live = s.kind === 'workspace' ? s.panes.filter(p => p.status === 'ready').length : s.kind === 'session' && s.status === 'ready' ? 1 : 0
  if (live) {
    closing.value = s
    closeOpen.value = true
  }
  else sessions.close(s)
}
function confirmClose() {
  closeOpen.value = false
  if (closing.value) sessions.close(closing.value)
}
// ---- Tab drag: reorder (outer quarters) or combine into a workspace (middle).
const dragKey = ref('')
function dropTab(ontoKey: string, where: 'before' | 'after' | 'onto') {
  const key = dragKey.value
  dragKey.value = ''
  if (!key || key === ontoKey) return
  if (where === 'onto') sessions.mergeTabs(key, ontoKey, hosts.hosts)
  else sessions.moveTab(key, ontoKey, where)
}
function tabMenu(s: Session): MenuItem[] {
  const others = sessions.sessions.filter(o => o.key !== s.key)
  const host = s.kind === 'session' && s.hostId ? hosts.hosts.find(h => h.id === s.hostId) : undefined
  return [
    ...(s.kind === 'session' && (host || !s.hostId) ? [{ label: 'Duplicate tab', icon: Copy, separator: true, onSelect: () => (host ? sessions.open(host) : sessions.openLocal()) }] : []),
    { label: 'Close others', disabled: !others.length, onSelect: () => others.forEach(requestClose) },
  ]
}
const closingCount = computed(() => (closing.value?.kind === 'workspace' ? closing.value.panes.length : 1))



// ---- Shortcuts. ⌘ on a Mac; Ctrl elsewhere, except inside a terminal where Ctrl belongs to the shell.
function go(to: typeof section.value) {
  sessions.activeKey = null
  section.value = to
  if (to === 'known-hosts') knownHosts.value?.refresh()
}
function newHost() {
  go('hosts')
  hosts.editing = 'new'
}
function onPaletteAction(name: string) {
  if (name === 'new-host') newHost()
  else if (name === 'new-tab') sessions.openNewTab()
  else if (name === 'local') sessions.openLocal()
  else if (name === 'lock') vault.lock()
  else if (name === 'sftp') sessions.activeKey = 'sftp'
  else go(name === 'vault' ? 'hosts' : name as typeof section.value)
}
/** ⌘D: another pane on the same server as the focused one. ⌘⇧D puts it directly below (one grid row down). */
function addDuplicatePane(ws: Session, below = false) {
  const host = hosts.hosts.find(h => h.id === ws.panes[ws.focus]?.hostId)
  if (!host) return
  const n = ws.panes.length
  const cols = n <= 1 ? 1 : n <= 4 ? 2 : n <= 9 ? 3 : 4
  sessions.addPane(ws, host, below ? ws.focus + cols : undefined)
}

function onShortcut(e: KeyboardEvent) {
  // ⌃Tab / ⌃⇧Tab cycle the tabs (the same on every platform).
  if (e.key === 'Tab' && e.ctrlKey && !e.metaKey && !e.altKey) {
    e.preventDefault()
    sessions.cycle(e.shiftKey ? -1 : 1)
    return
  }
  const combo = eventCombo(e)
  if (!combo) return
  // Outside a Mac, Ctrl combos inside a terminal belong to the shell. Shift/Alt ones are ours.
  const inTerminal = !!(e.target as HTMLElement | null)?.closest?.('.xterm')
  if (inTerminal && !navigator.platform.toLowerCase().includes('mac') && !/\+(Shift|Alt)\+/.test(combo)) return
  const tab = /^Mod\+([1-9])$/.exec(combo)
  if (tab) {
    sessions.activateNth(Number(tab[1]))
    e.preventDefault()
    return
  }
  const active = sessions.active
  const ws = active?.kind === 'workspace' ? active : null
  switch (actionFor(e, settings.combos)) {
    case 'palette': paletteOpen.value = !paletteOpen.value; break
    case 'new-tab': sessions.openNewTab(); break
    case 'close-tab': if (!active) return; requestClose(active); break
    case 'settings': go('settings'); break
    case 'side-panel': sessions.panelOpen = !sessions.panelOpen; break
    case 'broadcast': if (!ws) return; sessions.toggleBroadcast(ws); break
    case 'duplicate-pane': if (!ws) return; addDuplicatePane(ws); break
    case 'split-down': if (!ws) return; addDuplicatePane(ws, true); break
    case 'toggle-layout': if (!ws) return; sessions.toggleMode(ws); break
    case 'save-template': if (!ws) return; workspaceView.value?.[0]?.askSave(); break
    case 'new-host': newHost(); break
    default: return // includes `find`, which the terminal handles itself
  }
  e.preventDefault()
}
onMounted(() => window.addEventListener('keydown', onShortcut, true))
onBeforeUnmount(() => window.removeEventListener('keydown', onShortcut, true))
</script>

<template>
  <div class="flex h-screen flex-col bg-bg text-text">
    <TabBar mac @new="sessions.openNewTab()">
      <AppTab label="Vault" :active="sessions.activeKey === null" @select="sessions.activeKey = null">
        <template #leading><Vault class="size-4" aria-hidden="true" /></template>
      </AppTab>
      <AppTab label="SFTP" :active="sessions.activeKey === 'sftp'" @select="sessions.activeKey = 'sftp'">
        <template #leading><ArrowDownUp class="size-4" aria-hidden="true" /></template>
      </AppTab>
      <AppTab
        v-for="s in sessions.sessions"
        :key="s.key"
        :data-testid="tid(`tab-${s.key}`)"
        :label="s.label"
        :active="sessions.activeKey === s.key"
        :unsaved="s.kind === 'workspace' && s.dirty"
        closable
        movable
        :extra="tabMenu(s)"
        @select="sessions.activeKey = s.key"
        @close="requestClose(s)"
        @rename="(name: string) => sessions.rename(s, name)"
        @drag-start="dragKey = s.key"
        @drag-end="dragKey = ''"
        @drop-on="(where: 'before' | 'after' | 'onto') => dropTab(s.key, where)"
      >
        <template #leading>
          <StatusDot v-if="s.kind === 'session'" :status="paneDot(s)" :label="paneDotLabel(s, s.label)" />
          <template v-else-if="s.kind === 'workspace'">
            <LayoutGrid class="size-4 text-accent" aria-hidden="true" />
            <StatusDot :status="workspaceDot(s.panes)" :label="`${s.label}: ${workspaceDot(s.panes) === 'idle' ? 'all quiet' : workspaceDot(s.panes) === 'waiting' ? 'a pane is waiting for input' : workspaceDot(s.panes) === 'error' ? 'a pane failed or disconnected' : 'new output'}`" />
          </template>
        </template>
      </AppTab>
    </TabBar>

    <div class="relative min-h-0 flex-1">
      <!-- Terminals stay mounted (v-show), so scrollback survives switching tabs. -->
      <div v-for="s in sessions.sessions" v-show="sessions.activeKey === s.key" :key="s.key" class="absolute inset-0">
        <WorkspaceView v-if="s.kind === 'workspace'" ref="workspaceView" :ws="s" :active="sessions.activeKey === s.key" @edit-host="editHost" @new-host="newHost" />
        <SessionPane v-else :session="s" :active="sessions.activeKey === s.key" @edit-host="editHost" @new-host="newHost" />
      </div>

      <div v-show="sessions.activeKey === 'sftp'" class="absolute inset-0"><SftpView /></div>

      <div v-show="sessions.activeKey === null" class="flex size-full">
        <div class="flex w-[220px] shrink-0 flex-col border-r border-border bg-surface">
          <VaultNav :items="nav" :active="section" class="w-full border-r-0" @select="(id: string) => { section = id as typeof section; if (id === 'known-hosts') knownHosts?.refresh() }" />
          <div v-if="hosts.allTags.length" class="flex flex-1 flex-col gap-1.5 px-2 pt-4">
            <span class="px-2.5 text-2xs font-medium uppercase tracking-wider text-muted">Tags</span>
            <div class="flex flex-wrap gap-1.5 px-2.5">
              <ContextMenu v-for="t in hosts.allTags" :key="t">
                <ContextMenuTrigger as-child>
                  <button
                    type="button"
                    :aria-pressed="hosts.selectedTags.includes(t)"
                    :class="['h-[22px] rounded-full border px-2 text-xs', hosts.selectedTags.includes(t) ? 'border-accent bg-accent/10 text-text' : 'border-border bg-surface-hover text-muted hover:text-text']"
                    @click="section = 'hosts'; hosts.toggleTag(t)"
                  >
                    {{ t }}
                  </button>
                </ContextMenuTrigger>
                <ContextMenuContent>
                  <ContextMenuItem class="text-danger" @select="hosts.deleteTag(t)"><Trash2 />Delete tag</ContextMenuItem>
                </ContextMenuContent>
              </ContextMenu>
            </div>
          </div>
          <div v-else class="flex-1" />
          <div class="flex flex-col gap-0.5 p-2">
            <Button variant="ghost" class="justify-start text-muted" @click="vault.lock()"><Lock />Lock</Button>
            <Button variant="ghost" :class="['justify-start text-muted', section === 'settings' && 'bg-surface-hover text-text']" @click="go('settings')"><Settings />Settings</Button>
          </div>
        </div>

        <main class="flex min-w-0 flex-1 flex-col">
          <template v-if="section === 'known-hosts'">
            <PageToolbar title="Known hosts" />
            <KnownHostsList ref="knownHosts" />
          </template>
          <KeychainView v-else-if="section === 'keychain'" />
          <ForwardsView v-else-if="section === 'forwards'" />
          <SnippetsView v-else-if="section === 'snippets'" />
          <LogsView v-else-if="section === 'logs'" @settings="go('settings')" />
          <SettingsView v-else-if="section === 'settings'" />
          <HostsView v-else />
        </main>
      </div>
    </div>

    <CommandPalette v-model:open="paletteOpen" @action="onPaletteAction" />
    <DroppedSessionsCard />

    <AlertDialog v-model:open="closeOpen">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Close {{ closing?.label }}?</AlertDialogTitle>
          <AlertDialogDescription>This ends {{ closingCount === 1 ? 'the connection' : `all ${closingCount} connections` }}, and anything running in the shell with it.</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction :class="buttonVariants({ variant: 'destructive' })" @click="confirmClose">Close tab</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>

  </div>
</template>
