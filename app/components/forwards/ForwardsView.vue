<script setup lang="ts">
import { filterSort } from '~/utils/listView'
// S09: the rules, their live state, and a switch each.
import { Plus, Waypoints } from '@lucide/vue'
import type { ForwardSummary } from '~/types/bindings'
import { buttonVariants } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'

const forwards = useForwardsStore()

const listQuery = ref('')
const listDesc = ref(false)
const shownRules = computed(() => filterSort(forwards.rules, listQuery.value, listDesc.value, r => r.label || r.host_label, r => `${r.label} ${r.host_label} ${r.bind_port} ${r.dest_address ?? ''}`))

const sheetOpen = ref(false)
const editing = ref<ForwardSummary | null>(null)
function openSheet(r: ForwardSummary | null) {
  editing.value = r
  sheetOpen.value = true
}

const letter = (r: ForwardSummary) => ({ local: 'L', remote: 'R', dynamic: 'D' } as const)[r.kind]
const route = (r: ForwardSummary) => {
  const dest = `${r.dest_address}:${r.dest_port}`
  if (r.kind === 'dynamic') return `SOCKS on ${r.bind_address}:${r.bind_port} via ${r.host_label}`
  if (r.kind === 'remote') return `${r.host_label}:${r.bind_port} → this computer:${r.dest_port}`
  return `${r.bind_address}:${r.bind_port} → ${r.host_label} → ${dest}`
}

const toDelete = ref<ForwardSummary | null>(null)
const confirmOpen = ref(false)
function askDelete(r: ForwardSummary) {
  sheetOpen.value = false
  toDelete.value = r
  confirmOpen.value = true
}
async function confirmDelete() {
  const r = toDelete.value
  confirmOpen.value = false
  if (r) await forwards.remove(r)
}

const promptRule = computed(() => forwards.rules.find(r => r.id === forwards.prompt?.ruleId))
</script>

<template>
  <PageToolbar title="Port forwarding">
    <ListControls v-if="forwards.loaded && forwards.rules.length" v-model:query="listQuery" v-model:desc="listDesc" placeholder="Search rules" :count="shownRules.length" />
    <Button v-if="forwards.loaded && forwards.rules.length" @click="openSheet(null)"><Plus />New rule</Button>
  </PageToolbar>

  <div class="flex-1 overflow-auto p-6">
    <EmptyState v-if="forwards.loaded && !forwards.rules.length" :icon="Waypoints" title="No tunnels" text="Forward a remote port to your machine, or the other way round.">
      <Button @click="openSheet(null)"><Plus />New rule</Button>
    </EmptyState>
    <div v-else class="flex max-w-[760px] flex-col gap-2">
      <RuleCard
        v-for="r in shownRules"
        :key="r.id"
        :kind="letter(r)"
        :label="r.label"
        :route="route(r)"
        :state="r.state"
        :connections="r.connections"
        :error="r.error ?? undefined"
        @toggle="forwards.toggle(r)"
        @edit="openSheet(r)"
      />
    </div>
  </div>

  <ForwardSheet v-model:open="sheetOpen" :rule="editing" @delete="askDelete" />

  <!-- A rule that is starting may need a host key or password answered. -->
  <Dialog :open="!!forwards.prompt" @update:open="(v: boolean) => { if (!v) forwards.answer(false, null, false) }">
    <DialogContent class="sm:max-w-[420px]">
      <DialogHeader>
        <DialogTitle>Starting {{ promptRule?.label }}</DialogTitle>
        <DialogDescription>{{ promptRule?.host_label }} needs an answer before the tunnel can start.</DialogDescription>
      </DialogHeader>
      <PromptPanel v-if="forwards.prompt" :kind="forwards.prompt.kind" :host="promptRule?.host_label ?? ''" @reply="forwards.answer" />
    </DialogContent>
  </Dialog>

  <AlertDialog v-model:open="confirmOpen">
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>Delete {{ toDelete?.label }}?</AlertDialogTitle>
        <AlertDialogDescription>The tunnel stops if it is running.</AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel>Cancel</AlertDialogCancel>
        <AlertDialogAction :class="buttonVariants({ variant: 'destructive' })" @click="confirmDelete">Delete rule</AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
