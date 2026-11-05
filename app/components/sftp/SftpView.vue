<script setup lang="ts">
// S21 + S22: two panes and the transfer queue. Both panes stay mounted while you work elsewhere.
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { buttonVariants } from '@/components/ui/button'
import { formatMode } from '~/utils/format'

const sftp = useSftpStore()
const hosts = useHostsStore()

// The left pane starts on this computer, the first time the tab is used.
onMounted(() => {
  if (!sftp.panes[0].endpoint) sftp.showLocal(0)
})

// ---- small dialogs: new folder, rename, permissions, delete
const askName = ref<{ pane: number, kind: 'folder' | 'rename', from: string } | null>(null)
const nameOpen = ref(false)
const nameValue = ref('')
function newFolder(pane: number) {
  askName.value = { pane, kind: 'folder', from: '' }
  nameValue.value = ''
  nameOpen.value = true
}
function renameAsk(pane: number, name: string) {
  askName.value = { pane, kind: 'rename', from: name }
  nameValue.value = name
  nameOpen.value = true
}
async function submitName() {
  const a = askName.value
  nameOpen.value = false
  const name = nameValue.value.trim()
  if (!a || !name || name.includes('/') || name.includes('\\')) return
  if (a.kind === 'folder') await sftp.mkdir(a.pane, name)
  else if (name !== a.from) await sftp.rename(a.pane, a.from, name)
}

const chmodFor = ref<{ pane: number, name: string } | null>(null)
const chmodOpen = ref(false)
const chmodValue = ref('')
function chmodAsk(pane: number, name: string) {
  const e = sftp.panes[pane]!.entries.find(x => x.name === name)
  chmodFor.value = { pane, name }
  chmodValue.value = e?.mode == null ? '644' : e.mode.toString(8)
  chmodOpen.value = true
}
const chmodMode = computed(() => (/^[0-7]{3,4}$/.test(chmodValue.value) ? Number.parseInt(chmodValue.value, 8) : null))
async function submitChmod() {
  const c = chmodFor.value
  chmodOpen.value = false
  if (c && chmodMode.value !== null) await sftp.chmod(c.pane, c.name, chmodMode.value)
}

const removeFor = ref<{ pane: number, names: string[] } | null>(null)
const removeOpen = ref(false)
function removeAsk(pane: number, names: string[]) {
  removeFor.value = { pane, names }
  removeOpen.value = true
}
async function confirmRemove() {
  const r = removeFor.value
  removeOpen.value = false
  if (r) await sftp.remove(r.pane, r.names)
}
const removeLabel = computed(() => (removeFor.value?.names.length === 1 ? removeFor.value.names[0] : `${removeFor.value?.names.length} items`))
const promptHost = computed(() => hosts.hosts.find(h => h.id === sftp.panes[sftp.prompt?.pane ?? 0]?.hostId)?.label ?? sftp.panes[sftp.prompt?.pane ?? 0]?.label ?? '')
</script>

<template>
  <div class="flex size-full flex-col bg-bg">
    <div class="grid min-h-0 flex-1 grid-cols-2 gap-px bg-border">
      <SftpPane
        v-for="i in [0, 1]"
        :key="i"
        :index="i"
        @new-folder="newFolder(i)"
        @rename="n => renameAsk(i, n)"
        @chmod="n => chmodAsk(i, n)"
        @remove="n => removeAsk(i, n)"
      />
    </div>
    <TransferDrawer />

    <Dialog v-model:open="nameOpen">
      <DialogContent class="sm:max-w-[380px]">
        <DialogHeader><DialogTitle>{{ askName?.kind === 'folder' ? 'New folder' : 'Rename' }}</DialogTitle></DialogHeader>
        <form class="flex flex-col gap-3" @submit.prevent="submitName">
          <Input v-model="nameValue" aria-label="Name" autofocus autocomplete="off" spellcheck="false" />
          <div class="flex justify-end gap-2"><Button type="button" variant="secondary" @click="nameOpen = false">Cancel</Button><Button type="submit">{{ askName?.kind === 'folder' ? 'Create' : 'Rename' }}</Button></div>
        </form>
      </DialogContent>
    </Dialog>

    <Dialog v-model:open="chmodOpen">
      <DialogContent class="sm:max-w-[380px]">
        <DialogHeader>
          <DialogTitle>Permissions</DialogTitle>
          <DialogDescription>{{ chmodFor?.name }}</DialogDescription>
        </DialogHeader>
        <form class="flex flex-col gap-3" @submit.prevent="submitChmod">
          <div class="flex flex-col gap-1.5">
            <Label for="chmod-mode">Mode (octal)</Label>
            <Input id="chmod-mode" v-model="chmodValue" class="font-mono" autofocus autocomplete="off" :aria-invalid="chmodMode === null || undefined" />
            <p class="font-mono text-xs text-muted">{{ chmodMode === null ? 'Use 3 or 4 octal digits, like 644 or 0755.' : formatMode(chmodMode, 'file', false) }}</p>
          </div>
          <div class="flex justify-end gap-2"><Button type="button" variant="secondary" @click="chmodOpen = false">Cancel</Button><Button type="submit" :disabled="chmodMode === null">Apply</Button></div>
        </form>
      </DialogContent>
    </Dialog>

    <AlertDialog v-model:open="removeOpen">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete {{ removeLabel }}?</AlertDialogTitle>
          <AlertDialogDescription>This can't be undone. Folders are deleted with everything in them.</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction :class="buttonVariants({ variant: 'destructive' })" @click="confirmRemove">Delete</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>

    <!-- A server that is connecting may need a host key or password answered. -->
    <Dialog :open="!!sftp.prompt" @update:open="(v: boolean) => { if (!v) sftp.answer(false, null, false) }">
      <DialogContent class="sm:max-w-[420px]">
        <DialogHeader>
          <DialogTitle>Connecting to {{ promptHost }}</DialogTitle>
          <DialogDescription>It needs an answer before files can be shown.</DialogDescription>
        </DialogHeader>
        <PromptPanel v-if="sftp.prompt" :kind="sftp.prompt.kind" :host="promptHost" @reply="sftp.answer" />
      </DialogContent>
    </Dialog>
  </div>
</template>
