<script setup lang="ts">
// S02 Hosts. The full app shell (tabs, all vault sections) arrives in P7; until then this page
// carries a small Hosts-only sidebar so the layout already matches the canvas.
import { LayoutGrid, List, Lock, Plus, Server, Settings } from '@lucide/vue'
import type { HostSummary } from '~/types/bindings'
import { buttonVariants } from '@/components/ui/button'

const vault = useVaultStore()
const hosts = useHostsStore()
// `toDelete` stays set while the dialog closes, so the Delete click can still read it.
const toDelete = ref<HostSummary | null>(null)
const confirmOpen = ref(false)
function askDelete(h: HostSummary) {
  toDelete.value = h
  confirmOpen.value = true
}
const selected = ref<string | null>(null)

onMounted(() => hosts.refresh())
onBeforeUnmount(() => hosts.clear())

const nav = computed(() => [{ id: 'hosts', label: 'Hosts', icon: Server, count: hosts.hosts.length }])

// ⌘N adds a host from anywhere on this page (canvas: HostsEmpty).
function onKey(e: KeyboardEvent) {
  if ((e.metaKey || e.ctrlKey) && e.key === 'n') {
    e.preventDefault()
    hosts.editing = 'new'
  }
}
onMounted(() => window.addEventListener('keydown', onKey))
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))

async function confirmDelete() {
  const h = toDelete.value
  confirmOpen.value = false
  if (h) await hosts.remove(h)
}
</script>

<template>
  <div class="flex h-screen bg-bg text-text">
    <div class="flex w-[220px] shrink-0 flex-col border-r border-border bg-surface">
      <VaultNav :items="nav" active="hosts" class="w-full flex-1 border-r-0" />
      <div class="flex flex-col gap-0.5 p-2">
        <Button variant="ghost" class="justify-start text-muted" @click="vault.lock()"><Lock />Lock</Button>
        <Button variant="ghost" class="justify-start text-muted" disabled><Settings />Settings</Button>
      </div>
    </div>

    <main class="flex min-w-0 flex-1 flex-col">
      <PageToolbar title="Hosts">
        <template v-if="hosts.loaded && hosts.hosts.length">
          <SearchInput v-model="hosts.query" placeholder="Search hosts" :result-count="hosts.filtered.length" />
          <div role="group" aria-label="View" class="flex overflow-hidden rounded-md border border-border">
            <IconButton label="Grid view" :pressed="hosts.view === 'grid'" class="rounded-none" @click="hosts.view = 'grid'"><LayoutGrid /></IconButton>
            <IconButton label="List view" :pressed="hosts.view === 'list'" class="rounded-none" @click="hosts.view = 'list'"><List /></IconButton>
          </div>
          <Button @click="hosts.editing = 'new'"><Plus />New host</Button>
        </template>
      </PageToolbar>

      <div class="flex-1 overflow-auto p-6">
        <EmptyState v-if="hosts.loaded && !hosts.hosts.length" :icon="Server" title="No servers yet" text="Add one by hand, or bring them over from your SSH config.">
          <Button @click="hosts.editing = 'new'"><Plus />Add host</Button>
          <Button variant="secondary" disabled title="Arrives with import (P12)">Import</Button>
        </EmptyState>

        <p v-else-if="hosts.loaded && !hosts.filtered.length" class="py-8 text-center text-muted">No hosts match "{{ hosts.query }}".</p>

        <div
          v-else
          role="grid"
          aria-label="Hosts"
          :class="hosts.view === 'grid' ? 'grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-3' : 'flex flex-col gap-1.5'"
        >
          <HostCard
            v-for="h in hosts.filtered"
            :key="h.id"
            :label="h.label"
            :user="h.username ?? ''"
            :address="h.address"
            :port="h.port ?? undefined"
            :list="hosts.view === 'list'"
            :selected="selected === h.id"
            @select="selected = h.id"
            @edit="hosts.editing = h"
            @delete="askDelete(h)"
          />
        </div>
      </div>
    </main>

    <HostEditorSheet />

    <AlertDialog v-model:open="confirmOpen">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete {{ toDelete?.label }}?</AlertDialogTitle>
          <AlertDialogDescription>This removes the host and its saved password from this computer.</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction :class="buttonVariants({ variant: 'destructive' })" @click="confirmDelete">Delete host</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
