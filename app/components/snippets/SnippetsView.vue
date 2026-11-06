<script setup lang="ts">
import { filterSort } from '~/utils/listView'
import type { MenuItem } from '@/components/common/ItemMenuList.vue'
import { ContextMenu, ContextMenuContent, ContextMenuTrigger } from '@/components/ui/context-menu'
// S11: snippets grouped by package.
import { Code2, CopyPlus, FolderPlus, Pencil, Play, Plus, Trash2 } from '@lucide/vue'
import type { PackageSummary, SnippetSummary } from '~/types/bindings'
import { buttonVariants } from '@/components/ui/button'
import { DropdownMenuItem } from '@/components/ui/dropdown-menu'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'

const snippets = useSnippetsStore()
const hosts = useHostsStore()

const runOpen = ref(false)
const runSnippet = ref<SnippetSummary | null>(null)
function askRun(s: SnippetSummary) {
  runSnippet.value = s
  runOpen.value = true
}

const sheetOpen = ref(false)
const editing = ref<SnippetSummary | null>(null)
function openSheet(s: SnippetSummary | null) {
  editing.value = s
  sheetOpen.value = true
}

const listQuery = ref('')
const listDesc = ref(false)
const shown = computed(() => filterSort(snippets.snippets, listQuery.value, listDesc.value, s => s.label, s => `${s.label} ${s.script}`))
const groups = computed(() => {
  const out: { id: string | null, label: string, pkg: PackageSummary | null, items: SnippetSummary[] }[] = snippets.packages.map(p => ({
    id: p.id, label: p.label, pkg: p, items: shown.value.filter(s => s.package_id === p.id),
  }))
  const loose = shown.value.filter(s => !s.package_id)
  if (loose.length) out.push({ id: null, label: snippets.packages.length ? 'No package' : 'Snippets', pkg: null, items: loose })
  return out
})
const targets = (s: SnippetSummary) => (s.target_host_ids.length ? s.target_host_ids.map(id => hosts.hosts.find(h => h.id === id)?.label ?? '?').join(', ') : 'any')

// ---- package create / rename
const pkgOpen = ref(false)
const pkgEditing = ref<PackageSummary | null>(null)
const pkgLabel = ref('')
const pkgError = ref('')
function openPackage(p: PackageSummary | null) {
  pkgEditing.value = p
  pkgLabel.value = p?.label ?? ''
  pkgError.value = ''
  pkgOpen.value = true
}
async function savePackage() {
  pkgError.value = await snippets.savePackage(pkgEditing.value?.id ?? null, pkgLabel.value)
  if (!pkgError.value) pkgOpen.value = false
}

// ---- delete confirmations (targets stay set while the dialog closes)
const snToDelete = ref<SnippetSummary | null>(null)
const snConfirm = ref(false)
const pkToDelete = ref<PackageSummary | null>(null)
const pkConfirm = ref(false)
function askDelete(s: SnippetSummary) {
  sheetOpen.value = false
  snToDelete.value = s
  snConfirm.value = true
}
async function confirmSn() {
  const s = snToDelete.value
  snConfirm.value = false
  if (s) await snippets.remove(s)
}
async function confirmPk() {
  const p = pkToDelete.value
  pkConfirm.value = false
  if (p) await snippets.removePackage(p)
}

function snippetMenu(s: (typeof snippets.snippets)[number]): MenuItem[] {
  return [
    { label: 'Run…', icon: Play, onSelect: () => askRun(s) },
    { label: 'Edit', icon: Pencil, onSelect: () => openSheet(s) },
    { label: 'Duplicate', icon: CopyPlus, onSelect: () => snippets.duplicate(s) },
    { label: 'Delete', icon: Trash2, danger: true, separator: true, onSelect: () => askDelete(s) },
  ]
}
</script>

<template>
  <PageToolbar title="Snippets">
    <ListControls v-if="snippets.loaded && (snippets.snippets.length || snippets.packages.length)" v-model:query="listQuery" v-model:desc="listDesc" placeholder="Search snippets" />
    <SplitButton v-if="snippets.loaded && (snippets.snippets.length || snippets.packages.length)" label="New snippet" @click="openSheet(null)">
      <template #icon><Plus /></template>
      <DropdownMenuItem @select="openPackage(null)"><FolderPlus />New package</DropdownMenuItem>
    </SplitButton>
  </PageToolbar>

  <div class="flex-1 overflow-auto p-6">
    <EmptyState v-if="snippets.loaded && !snippets.snippets.length && !snippets.packages.length" :icon="Code2" title="No snippets" text="Save commands you type often and run them on many servers.">
      <Button @click="openSheet(null)"><Plus />New snippet</Button>
    </EmptyState>

    <div v-else class="flex max-w-[760px] flex-col gap-5">
      <section v-for="g in groups" :key="g.id ?? 'none'" class="flex flex-col gap-1.5">
        <h2 class="flex items-center gap-2 text-xs font-medium text-muted">
          {{ g.label }}
          <template v-if="g.pkg">
            <IconButton label="Rename package" size="xs" @click="openPackage(g.pkg)"><Pencil /></IconButton>
            <IconButton label="Delete package" size="xs" @click="pkToDelete = g.pkg; pkConfirm = true"><Trash2 /></IconButton>
          </template>
        </h2>
        <p v-if="!g.items.length" class="px-1 text-sm text-muted">Empty. Choose this package when you edit a snippet.</p>
        <ContextMenu v-for="s in g.items" :key="s.id">
          <ContextMenuTrigger as-child>
        <div
          class="flex items-center gap-3 rounded-lg border border-border bg-surface px-3 py-2.5 hover:border-muted/40 hover:bg-surface-raised"
        >
          <button type="button" class="flex min-w-0 flex-1 flex-col text-left" @click="openSheet(s)">
            <span class="font-medium">{{ s.label }}</span>
            <span class="truncate font-mono text-xs text-muted">{{ s.script.split('\n')[0] }}</span>
          </button>
          <span class="shrink-0 text-xs text-muted">{{ targets(s) }}</span>
          <Button variant="secondary" size="sm" @click="askRun(s)"><Play />Run</Button>
        </div>
          </ContextMenuTrigger>
          <ContextMenuContent>
            <ItemMenuList :items="snippetMenu(s)" mode="context" />
          </ContextMenuContent>
        </ContextMenu>
      </section>
    </div>
  </div>

  <SnippetSheet v-model:open="sheetOpen" :snippet="editing" @delete="askDelete" />
  <RunOnHostsDialog v-model:open="runOpen" :snippet="runSnippet" />

  <Dialog v-model:open="pkgOpen">
    <DialogContent class="sm:max-w-[380px]">
      <DialogHeader><DialogTitle>{{ pkgEditing ? 'Rename package' : 'New package' }}</DialogTitle></DialogHeader>
      <form class="flex flex-col gap-3" @submit.prevent="savePackage">
        <Input v-model="pkgLabel" aria-label="Package name" placeholder="Deploy" autofocus autocomplete="off" />
        <p v-if="pkgError" role="alert" class="text-sm text-danger">{{ pkgError }}</p>
        <div class="flex justify-end gap-2"><Button type="button" variant="secondary" @click="pkgOpen = false">Cancel</Button><Button type="submit">Save</Button></div>
      </form>
    </DialogContent>
  </Dialog>

  <AlertDialog v-model:open="snConfirm">
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>Delete {{ snToDelete?.label }}?</AlertDialogTitle>
        <AlertDialogDescription>Hosts that run it when they connect will stop doing that.</AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel>Cancel</AlertDialogCancel>
        <AlertDialogAction :class="buttonVariants({ variant: 'destructive' })" @click="confirmSn">Delete snippet</AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>

  <AlertDialog v-model:open="pkConfirm">
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>Delete {{ pkToDelete?.label }}?</AlertDialogTitle>
        <AlertDialogDescription>The snippets in it are kept.</AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel>Cancel</AlertDialogCancel>
        <AlertDialogAction :class="buttonVariants({ variant: 'destructive' })" @click="confirmPk">Delete package</AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
