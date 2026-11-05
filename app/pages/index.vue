<script setup lang="ts">
// S02 Hosts. The full app shell (tabs, all vault sections) arrives in P7; until then this page
// carries a small Hosts-only sidebar so the layout already matches the canvas.
import { ArrowDownUp, Code2, KeyRound, Lock, Server, Settings, ShieldCheck, Vault, Waypoints } from '@lucide/vue'
import { buttonVariants } from '@/components/ui/button'
import type { Session } from '~/stores/sessions'

const vault = useVaultStore()
const hosts = useHostsStore()
const sessions = useSessionsStore()
const keychain = useKeychainStore()
const snippetsStore = useSnippetsStore()
const forwardsStore = useForwardsStore()
const sftpStore = useSftpStore()
const settings = useSettingsStore()
const section = ref<'hosts' | 'keychain' | 'forwards' | 'snippets' | 'known-hosts' | 'settings'>('hosts')
const paletteOpen = ref(false)
const knownHosts = ref<{ refresh: () => void } | null>(null)
onMounted(() => {
  hosts.refresh()
  keychain.refresh()
  snippetsStore.refresh()
  forwardsStore.refresh()
  forwardsStore.watch()
  settings.load()
})
onBeforeUnmount(() => {
  hosts.clear()
  keychain.clear()
  snippetsStore.clear()
  forwardsStore.clear()
  sftpStore.clear()
  sessions.clear()
})

const nav = computed(() => [
  { id: 'hosts', label: 'Hosts', icon: Server, count: hosts.hosts.length },
  { id: 'keychain', label: 'Keychain', icon: KeyRound, count: keychain.keys.length + keychain.identities.length },
  { id: 'forwards', label: 'Port forwarding', icon: Waypoints, count: forwardsStore.rules.length },
  { id: 'snippets', label: 'Snippets', icon: Code2, count: snippetsStore.snippets.length },
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
  if (s.kind === 'session' && s.status === 'ready') {
    closing.value = s
    closeOpen.value = true
  }
  else sessions.close(s)
}
function confirmClose() {
  closeOpen.value = false
  if (closing.value) sessions.close(closing.value)
}
const statusDot = (s: Session) => (s.status === 'ready' ? 'unread' : s.status === 'error' ? 'error' : s.status === 'disconnected' ? 'idle' : 'waiting')
const statusLabel = (s: Session) => ({ ready: 'Connected', error: 'Connection failed', disconnected: 'Disconnected', connecting: 'Connecting', authenticating: 'Signing in' })[s.status]



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
  else if (name === 'lock') vault.lock()
  else if (name === 'sftp') sessions.activeKey = 'sftp'
  else go(name === 'vault' ? 'hosts' : name as typeof section.value)
}
function onShortcut(e: KeyboardEvent) {
  const inTerminal = !!(e.target as HTMLElement | null)?.closest?.('.xterm')
  if (!(e.metaKey || (e.ctrlKey && !inTerminal))) return
  const k = e.key.toLowerCase()
  if (k === 'k') paletteOpen.value = !paletteOpen.value
  else if (k === 't') sessions.openNewTab()
  else if (k === 'w' && sessions.active) requestClose(sessions.active)
  else if (k === ',') go('settings')
  else if (k === '.') sessions.panelOpen = !sessions.panelOpen
  else if (k === 'n') newHost()
  else if (/^[1-9]$/.test(k)) sessions.activateNth(Number(k))
  else return
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
        :label="s.label"
        :active="sessions.activeKey === s.key"
        closable
        @select="sessions.activeKey = s.key"
        @close="requestClose(s)"
      >
        <template #leading><StatusDot v-if="s.kind === 'session'" :status="statusDot(s)" :label="statusLabel(s)" /></template>
      </AppTab>
    </TabBar>

    <div class="relative min-h-0 flex-1">
      <!-- Terminals stay mounted (v-show), so scrollback survives switching tabs. -->
      <div v-for="s in sessions.sessions" v-show="sessions.activeKey === s.key" :key="s.key" class="absolute inset-0">
        <SessionPane :session="s" :active="sessions.activeKey === s.key" @edit-host="editHost" @new-host="newHost" />
      </div>

      <div v-show="sessions.activeKey === 'sftp'" class="absolute inset-0"><SftpView /></div>

      <div v-show="sessions.activeKey === null" class="flex size-full">
        <div class="flex w-[220px] shrink-0 flex-col border-r border-border bg-surface">
          <VaultNav :items="nav" :active="section" class="w-full border-r-0" @select="(id: string) => { section = id as typeof section; if (id === 'known-hosts') knownHosts?.refresh() }" />
          <div v-if="hosts.allTags.length" class="flex flex-1 flex-col gap-1.5 px-2 pt-4">
            <span class="px-2.5 text-2xs font-medium uppercase tracking-wider text-muted">Tags</span>
            <div class="flex flex-wrap gap-1.5 px-2.5">
              <button
                v-for="t in hosts.allTags"
                :key="t"
                type="button"
                :aria-pressed="hosts.selectedTags.includes(t)"
                :class="['h-[22px] rounded-full border px-2 text-xs', hosts.selectedTags.includes(t) ? 'border-accent bg-accent/10 text-text' : 'border-border bg-surface-hover text-muted hover:text-text']"
                @click="section = 'hosts'; hosts.toggleTag(t)"
              >
                {{ t }}
              </button>
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
          <SettingsView v-else-if="section === 'settings'" />
          <HostsView v-else />
        </main>
      </div>
    </div>

    <CommandPalette v-model:open="paletteOpen" @action="onPaletteAction" />

    <AlertDialog v-model:open="closeOpen">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Close {{ closing?.label }}?</AlertDialogTitle>
          <AlertDialogDescription>This ends the connection, and anything running in the shell with it.</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction :class="buttonVariants({ variant: 'destructive' })" @click="confirmClose">Close tab</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>

  </div>
</template>
