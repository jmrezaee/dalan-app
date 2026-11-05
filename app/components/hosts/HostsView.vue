<script setup lang="ts">
// S02: groups first, then the hosts in the group you are in. Search and tag filters look at every host.
import { ChevronRight, FolderPlus, LayoutGrid, List, Plus, Server } from '@lucide/vue'
import type { GroupSummary, HostSummary } from '~/types/bindings'
import { buttonVariants } from '@/components/ui/button'
import { DropdownMenuItem } from '@/components/ui/dropdown-menu'

const hosts = useHostsStore()
const sessions = useSessionsStore()
const selected = ref<string | null>(null)

// ---- delete confirmations. The target stays set while the dialog closes, so the click can still read it.
const hostToDelete = ref<HostSummary | null>(null)
const hostConfirm = ref(false)
const groupToDelete = ref<GroupSummary | null>(null)
const groupConfirm = ref(false)
function askDeleteHost(h: HostSummary) {
  hostToDelete.value = h
  hostConfirm.value = true
}
function askDeleteGroup(g: GroupSummary) {
  groupToDelete.value = g
  groupConfirm.value = true
}
async function confirmDeleteHost() {
  const h = hostToDelete.value
  hostConfirm.value = false
  if (h) await hosts.remove(h)
}
async function confirmDeleteGroup() {
  const g = groupToDelete.value
  groupConfirm.value = false
  if (g) await hosts.removeGroup(g)
}
const upTo = computed(() => groupToDelete.value?.parent_id ? hosts.groupLabel(groupToDelete.value.parent_id) : 'the top level')

// ---- F04: drag a host onto a group or a breadcrumb.
const dragging = ref<string | null>(null)
const dropOn = ref<string | null>(null) // group id, or 'root'
function startDrag(e: DragEvent, h: HostSummary) {
  dragging.value = h.id
  e.dataTransfer?.setData('text/plain', h.id)
  if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move'
}
function over(e: DragEvent, target: string) {
  if (!dragging.value) return
  e.preventDefault() // allows the drop
  dropOn.value = target
}
async function dropped(target: string) {
  const id = dragging.value
  dragging.value = dropOn.value = null
  if (!id) return
  const groupId = target === 'root' ? null : target
  if (hosts.hosts.find(h => h.id === id)?.group_id === groupId) return
  await hosts.move([id], groupId)
}

function newHost() {
  hosts.editing = 'new'
}
// ⌘N adds a host from anywhere on this page (canvas: HostsEmpty), but not while a terminal is showing.
function onKey(e: KeyboardEvent) {
  if ((e.metaKey || e.ctrlKey) && e.key === 'n' && sessions.activeKey === null) {
    e.preventDefault()
    newHost()
  }
}
onMounted(() => window.addEventListener('keydown', onKey))
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))

const empty = computed(() => hosts.loaded && !hosts.hosts.length && !hosts.groups.length)
const nothingHere = computed(() => hosts.loaded && !empty.value && !hosts.childGroups.length && !hosts.visibleHosts.length)
</script>

<template>
  <PageToolbar title="Hosts">
    <template #title>
      <nav aria-label="Group" class="flex min-w-0 items-center gap-1 text-xl">
        <component
          :is="hosts.breadcrumb.length || hosts.searching ? 'button' : 'h1'"
          :type="hosts.breadcrumb.length || hosts.searching ? 'button' : undefined"
          :class="['rounded px-1', dropOn === 'root' && 'bg-accent/10 outline outline-accent', (hosts.breadcrumb.length || hosts.searching) && 'text-muted hover:text-text']"
          @click="hosts.currentGroupId = null; hosts.query = ''; hosts.selectedTags = []"
          @dragover="over($event, 'root')"
          @dragleave="dropOn = null"
          @drop.prevent="dropped('root')"
        >
          Hosts
        </component>
        <template v-for="(g, i) in hosts.breadcrumb" :key="g.id">
          <ChevronRight class="size-4 shrink-0 text-muted" aria-hidden="true" />
          <component
            :is="i < hosts.breadcrumb.length - 1 ? 'button' : 'h1'"
            :type="i < hosts.breadcrumb.length - 1 ? 'button' : undefined"
            :class="['truncate rounded px-1', dropOn === g.id && 'bg-accent/10 outline outline-accent', i < hosts.breadcrumb.length - 1 && 'text-muted hover:text-text']"
            @click="hosts.currentGroupId = g.id"
            @dragover="over($event, g.id)"
            @dragleave="dropOn = null"
            @drop.prevent="dropped(g.id)"
          >
            {{ g.label }}
          </component>
        </template>
      </nav>
    </template>
    <template v-if="hosts.loaded && !empty">
      <SearchInput v-model="hosts.query" placeholder="Search hosts" :result-count="hosts.visibleHosts.length" />
      <div role="group" aria-label="View" class="flex overflow-hidden rounded-md border border-border">
        <IconButton label="Grid view" :pressed="hosts.view === 'grid'" class="rounded-none" @click="hosts.view = 'grid'"><LayoutGrid /></IconButton>
        <IconButton label="List view" :pressed="hosts.view === 'list'" class="rounded-none" @click="hosts.view = 'list'"><List /></IconButton>
      </div>
      <SplitButton label="New host" @click="newHost">
        <template #icon><Plus /></template>
        <DropdownMenuItem @select="hosts.editingGroup = 'new'"><FolderPlus />New group</DropdownMenuItem>
        <DropdownMenuItem disabled>Import from SSH config (P12)</DropdownMenuItem>
      </SplitButton>
    </template>
  </PageToolbar>

  <div class="flex-1 overflow-auto p-6">
    <EmptyState v-if="empty" :icon="Server" title="No servers yet" text="Add one by hand, or bring them over from your SSH config.">
      <Button @click="newHost"><Plus />Add host</Button>
      <Button variant="secondary" disabled title="Arrives with import (P12)">Import</Button>
    </EmptyState>

    <div v-else class="flex flex-col gap-5">
      <section v-if="hosts.childGroups.length" class="flex flex-col gap-2.5" aria-label="Groups">
        <h2 class="text-xs font-medium text-muted">Groups</h2>
        <div class="grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-3">
          <GroupCard
            v-for="g in hosts.childGroups"
            :key="g.id"
            :label="g.label"
            :hosts="g.host_count"
            :groups="g.group_count"
            :drop-target="dropOn === g.id"
            @open="hosts.currentGroupId = g.id"
            @edit="hosts.editingGroup = g"
            @delete="askDeleteGroup(g)"
            @dragover="over($event, g.id)"
            @dragleave="dropOn = null"
            @drop="dropped(g.id)"
          />
        </div>
      </section>

      <section v-if="hosts.visibleHosts.length" class="flex flex-col gap-2.5">
        <h2 v-if="hosts.childGroups.length || hosts.searching" class="text-xs font-medium text-muted">
          {{ hosts.searching ? 'Matching hosts' : (hosts.currentGroup?.label ?? 'Not in a group') }}
        </h2>
        <div role="grid" aria-label="Hosts" :class="hosts.view === 'grid' ? 'grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-3' : 'flex flex-col gap-1.5'">
          <HostCard
            v-for="h in hosts.visibleHosts"
            :key="h.id"
            :label="h.label"
            :user="h.effective_username ?? ''"
            :address="h.address"
            :port="h.effective_port"
            :tags="h.tags"
            :list="hosts.view === 'list'"
            :selected="selected === h.id"
            :dragging="dragging === h.id"
            @select="selected = h.id"
            @edit="hosts.editing = h"
            @open="sessions.open(h)"
            @delete="askDeleteHost(h)"
            @dragstart="startDrag($event, h)"
            @dragend="dragging = dropOn = null"
          />
        </div>
      </section>

      <p v-if="nothingHere" class="py-8 text-center text-muted">
        <template v-if="hosts.searching">No hosts match.</template>
        <template v-else>Nothing in this group yet. Add a host, or drag one here from the top level.</template>
      </p>
    </div>
  </div>

  <HostEditorSheet />
  <GroupEditorSheet />

  <AlertDialog v-model:open="hostConfirm">
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>Delete {{ hostToDelete?.label }}?</AlertDialogTitle>
        <AlertDialogDescription>This removes the host and its saved password from this computer.</AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel>Cancel</AlertDialogCancel>
        <AlertDialogAction :class="buttonVariants({ variant: 'destructive' })" @click="confirmDeleteHost">Delete host</AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>

  <AlertDialog v-model:open="groupConfirm">
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>Delete {{ groupToDelete?.label }}?</AlertDialogTitle>
        <AlertDialogDescription>Its hosts and groups move up to {{ upTo }}. Defaults set on this group, like its username and password, are lost.</AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel>Cancel</AlertDialogCancel>
        <AlertDialogAction :class="buttonVariants({ variant: 'destructive' })" @click="confirmDeleteGroup">Delete group</AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
