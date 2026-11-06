<script setup lang="ts">
import { filterSort } from '~/utils/listView'
// S05: keys and identities, with the dialogs that make them.
import { Download, KeyRound, Plus, Trash2, UserRound, Pencil } from '@lucide/vue'
import type { IdentitySummary, KeySummary } from '~/types/bindings'
import { buttonVariants } from '@/components/ui/button'
import { DropdownMenuItem } from '@/components/ui/dropdown-menu'

const keychain = useKeychainStore()

const generateOpen = ref(false)
const importOpen = ref(false)
const editKeyOpen = ref(false)
const exportOpen = ref(false)
const identityOpen = ref(false)
const certOpen = ref(false)
const activeKey = ref<KeySummary | null>(null)
const activeIdentity = ref<IdentitySummary | null>(null)

const keyToDelete = ref<KeySummary | null>(null)
const identityToDelete = ref<IdentitySummary | null>(null)
const keyConfirm = ref(false)
const identityConfirm = ref(false)

const listQuery = ref('')
const listDesc = ref(false)
const shownKeys = computed(() => filterSort(keychain.keys, listQuery.value, listDesc.value, k => k.label, k => `${k.label} ${k.fingerprint}`))
const shownIdentities = computed(() => filterSort(keychain.identities, listQuery.value, listDesc.value, i => i.label, i => `${i.label} ${i.username}`))
const empty = computed(() => keychain.loaded && !keychain.keys.length && !keychain.identities.length)

const typeLabel = (k: KeySummary) => (k.key_type === 'rsa' ? `RSA ${k.bits ?? ''}`.trim() : k.key_type.toUpperCase())
function install(k: KeySummary) {
  activeKey.value = k
  exportOpen.value = true
}
function attachCert(k: KeySummary) {
  activeKey.value = k
  certOpen.value = true
}
function editKey(k: KeySummary) {
  activeKey.value = k
  editKeyOpen.value = true
}
function newIdentity(i: IdentitySummary | null = null) {
  activeIdentity.value = i
  identityOpen.value = true
}
function askDeleteKey(k: KeySummary) {
  keyToDelete.value = k
  keyConfirm.value = true
}
function askDeleteIdentity(i: IdentitySummary) {
  identityToDelete.value = i
  identityConfirm.value = true
}
async function confirmDeleteKey() {
  const k = keyToDelete.value
  keyConfirm.value = false
  if (k) await keychain.removeKey(k)
}
async function confirmDeleteIdentity() {
  const i = identityToDelete.value
  identityConfirm.value = false
  if (i) await keychain.removeIdentity(i)
}
</script>

<template>
  <PageToolbar title="Keychain">
    <ListControls v-if="keychain.loaded && !empty" v-model:query="listQuery" v-model:desc="listDesc" placeholder="Search keys and identities" :count="shownKeys.length + shownIdentities.length" />
    <SplitButton v-if="keychain.loaded && !empty" label="Generate key" @click="generateOpen = true">
      <template #icon><Plus /></template>
      <DropdownMenuItem @select="importOpen = true"><Download />Import key…</DropdownMenuItem>
      <DropdownMenuItem @select="newIdentity()"><UserRound />New identity</DropdownMenuItem>
    </SplitButton>
  </PageToolbar>

  <div class="flex-1 overflow-auto p-6">
    <EmptyState v-if="empty" :icon="KeyRound" title="Your keychain is empty" text="Generate a key or import one you already use.">
      <Button @click="generateOpen = true"><Plus />Generate key</Button>
      <Button variant="secondary" @click="importOpen = true">Import key</Button>
    </EmptyState>

    <div v-else class="flex flex-col gap-6">
      <section class="flex flex-col gap-2.5" aria-label="Keys">
        <h2 class="text-xs font-medium text-muted">Keys</h2>
        <p v-if="!keychain.keys.length" class="text-sm text-muted">No keys yet.</p>
        <div v-else class="grid grid-cols-[repeat(auto-fill,minmax(340px,1fr))] gap-3">
          <KeyCard
            v-for="k in shownKeys"
            :key="k.id"
            :label="k.label"
            :type="typeLabel(k)"
            :fingerprint="k.fingerprint"
            :encrypted="k.encrypted"
            :passphrase-saved="k.passphrase_saved"
            :certificate="k.certificate"
            :certificate-note="k.certificate_note"
            @certificate="attachCert(k)"
            @remove-certificate="keychain.setCertificate(k.id, null, null)"
            @copy-public="keychain.copyPublic(k)"
            @install="install(k)"
            @edit="editKey(k)"
            @delete="askDeleteKey(k)"
          />
        </div>
      </section>

      <section class="flex flex-col gap-2.5" aria-label="Identities">
        <div class="flex items-center justify-between">
          <h2 class="text-xs font-medium text-muted">Identities</h2>
          <Button variant="ghost" size="sm" @click="newIdentity()"><Plus />New identity</Button>
        </div>
        <p v-if="!keychain.identities.length" class="text-sm text-muted">An identity is a username plus a password or key, shared by many hosts.</p>
        <div v-else class="grid grid-cols-[repeat(auto-fill,minmax(340px,1fr))] gap-3">
          <div v-for="i in shownIdentities" :key="i.id" class="group flex items-center gap-3 rounded-lg border border-border bg-surface p-3 hover:border-muted/40 hover:bg-surface-raised">
            <div class="grid size-8 shrink-0 place-items-center rounded-md bg-surface-hover"><UserRound class="size-4" aria-hidden="true" /></div>
            <div class="min-w-0 flex-1">
              <div class="truncate text-base">{{ i.label }}</div>
              <div class="truncate text-xs text-muted"><span class="font-mono">{{ i.username }}</span> · {{ i.key_id ? `key ${keychain.keyLabel(i.key_id) ?? ''}` : i.has_password ? 'password' : 'asks for a password' }}</div>
            </div>
            <IconButton label="Edit identity" size="xs" @click="newIdentity(i)"><Pencil /></IconButton>
            <IconButton label="Delete identity" size="xs" @click="askDeleteIdentity(i)"><Trash2 /></IconButton>
          </div>
        </div>
      </section>
    </div>
  </div>

  <GenerateKeyDialog v-model:open="generateOpen" />
  <ImportKeyDialog v-model:open="importOpen" />
  <KeyEditDialog v-model:open="editKeyOpen" :key-item="activeKey" />
  <CertificateDialog v-model:open="certOpen" :key-item="activeKey" />
  <ExportKeyDialog v-model:open="exportOpen" :key-item="activeKey" />
  <IdentitySheet v-model:open="identityOpen" :identity="activeIdentity" />

  <AlertDialog v-model:open="keyConfirm">
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>Delete {{ keyToDelete?.label }}?</AlertDialogTitle>
        <AlertDialogDescription>The private key is removed from this computer and can't be recovered. Hosts that use it fall back to the sign-in they inherit.</AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel>Cancel</AlertDialogCancel>
        <AlertDialogAction :class="buttonVariants({ variant: 'destructive' })" @click="confirmDeleteKey">Delete key</AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>

  <AlertDialog v-model:open="identityConfirm">
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>Delete {{ identityToDelete?.label }}?</AlertDialogTitle>
        <AlertDialogDescription>Hosts and groups that use it fall back to the sign-in they inherit.</AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel>Cancel</AlertDialogCancel>
        <AlertDialogAction :class="buttonVariants({ variant: 'destructive' })" @click="confirmDeleteIdentity">Delete identity</AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
