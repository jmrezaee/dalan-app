<script setup lang="ts">
// S02 Hosts. The full app shell (tabs, all vault sections) arrives in P7; until then this page
// carries a small Hosts-only sidebar so the layout already matches the canvas.
import { Lock, Server, Settings, ShieldCheck, Vault } from '@lucide/vue'
import { buttonVariants } from '@/components/ui/button'
import type { Session } from '~/stores/sessions'

const vault = useVaultStore()
const hosts = useHostsStore()
const sessions = useSessionsStore()
const section = ref<'hosts' | 'known-hosts'>('hosts')
const knownHosts = ref<{ refresh: () => void } | null>(null)
onMounted(() => hosts.refresh())
onBeforeUnmount(() => {
  hosts.clear()
  sessions.clear()
})

const nav = computed(() => [
  { id: 'hosts', label: 'Hosts', icon: Server, count: hosts.hosts.length },
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
  if (s.status === 'ready') {
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


</script>

<template>
  <div class="flex h-screen flex-col bg-bg text-text">
    <TabBar mac @new="sessions.activeKey = null">
      <AppTab label="Vault" :active="sessions.activeKey === null" @select="sessions.activeKey = null">
        <template #leading><Vault class="size-4" aria-hidden="true" /></template>
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
        <template #leading><StatusDot :status="statusDot(s)" :label="statusLabel(s)" /></template>
      </AppTab>
    </TabBar>

    <div class="relative min-h-0 flex-1">
      <!-- Terminals stay mounted (v-show), so scrollback survives switching tabs. -->
      <div v-for="s in sessions.sessions" v-show="sessions.activeKey === s.key" :key="s.key" class="absolute inset-0">
        <SessionPane :session="s" :active="sessions.activeKey === s.key" @edit-host="editHost" />
      </div>

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
            <Button variant="ghost" class="justify-start text-muted" disabled><Settings />Settings</Button>
          </div>
        </div>

        <main class="flex min-w-0 flex-1 flex-col">
          <template v-if="section === 'known-hosts'">
            <PageToolbar title="Known hosts" />
            <KnownHostsList ref="knownHosts" />
          </template>
          <HostsView v-else />
        </main>
      </div>
    </div>

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
