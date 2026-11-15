<script setup lang="ts">
// S28: the Browser view of a Redis tab: the key tree on the left, the open key's value on the right.
// R3 adds the editors. In Read-only mode (or on a replica) every add, edit and delete control is hidden; whatever
// the mode, each change is sent to the write gate in Rust, which decides (run, confirm, refuse).
import { CircleAlert, Database } from '@lucide/vue'
import { buttonVariants } from '@/components/ui/button'
import type { RedisWriteMode } from '~/types/bindings'
import { bytesText, count, parsedJson, textBytes, TYPE_UNIT } from '~/utils/redisBytes'

const props = defineProps<{ tabKey: string, separator: string, mode: RedisWriteMode, readOnly: boolean }>()
const writable = computed(() => !props.readOnly)
const saveLabel = computed(() => (props.mode === 'open' ? 'Save' : 'Review change'))
const store = useRedisStore(props.tabKey)
watchEffect(() => store.setSeparator(props.separator))

const v = computed(() => store.value)
/** How many items are loaded, and what they are called, for "Showing 500 of 48,210". */
const loaded = computed(() => {
  const x = v.value
  if (!x) return 0
  return x.type === 'hash' ? x.fields.length : x.type === 'list' ? x.items.length : x.type === 'set' || x.type === 'zset' ? x.members.length : x.type === 'stream' ? x.entries.length : 0
})
const total = computed(() => store.keyInfo?.length ?? 0)
const unit = computed(() => TYPE_UNIT[store.keyInfo?.key_type ?? ''])
const isCollection = computed(() => !!v.value && ['hash', 'list', 'set', 'zset', 'stream'].includes(v.value.type))
const empty = computed(() => isCollection.value && loaded.value === 0 && !store.valueLoading)
const jsonValue = computed(() => (v.value?.type === 'json' ? parsedJson({ kind: 'text', text: v.value.text }) ?? v.value.text : undefined))
const jsonMode = ref('tree')

// ---- Changes to the open key itself (F15). Each is one write for Rust's gate.
const newKeyOpen = ref(false)
const keyName = computed(() => (store.selectedKey ? bytesText(store.selectedKey) : ''))
function saveString(text: string) {
  if (store.selectedKey) void store.write({ op: 'set_string', key: store.selectedKey, value: textBytes(text) }, { title: 'Change this value?', action: 'Change' })
}
function saveJson(text: string) {
  if (store.selectedKey) void store.write({ op: 'set_json', key: store.selectedKey, text }, { title: 'Change this value?', action: 'Change' })
}
function rename(to: string) {
  if (store.selectedKey) void store.write({ op: 'rename', key: store.selectedKey, to: textBytes(to) }, { title: 'Rename this key?', action: 'Rename' })
}
function expire(ttlMs: number | null) {
  if (store.selectedKey) void store.write({ op: 'expire', key: store.selectedKey, ttl_ms: ttlMs }, ttlMs === null ? { title: 'Remove the expiry?', action: 'Remove expiry' } : { title: 'Set this expiry?', action: 'Set expiry' })
}
// Open mode sends a delete at once, so the browser asks first. In Confirm mode the gate's own dialog is the question.
const deleteAsk = ref(false)
function deleteKey() {
  deleteAsk.value = false
  if (store.selectedKey) void store.write({ op: 'delete', key: store.selectedKey }, { title: `Delete ${keyName.value}?`, action: 'Delete key', danger: true })
}
const askDelete = () => (props.mode === 'open' ? (deleteAsk.value = true) : deleteKey())
</script>

<template>
  <div class="flex min-h-0 flex-1" :data-testid="tid('redis-browser')">
    <KeyTree :tab-key="tabKey" :separator="separator" :writable="writable" @new-key="newKeyOpen = true" />

    <section v-if="store.selectedKey" :aria-label="`Key ${store.keyInfo ? '' : 'loading'}`" class="flex min-w-0 flex-1 flex-col" :data-testid="tid('redis-value')">
      <KeyHeader v-if="store.keyInfo && store.keyInfo.key_type !== 'none'" :info="store.keyInfo" :writable="writable" :save-label="saveLabel" @reload="store.reloadKey()" @rename="rename" @expire="expire" @delete="askDelete" />

      <div v-if="store.valueError" class="grid flex-1 place-items-center p-6">
        <div class="flex w-[440px] max-w-full items-center gap-2.5 rounded-lg border border-border bg-surface px-3 py-2.5" role="alert">
          <CircleAlert class="size-4 shrink-0 text-danger" aria-hidden="true" />
          <span class="min-w-0 flex-1 text-sm">{{ store.valueError.message }}</span>
          <Button v-if="store.valueError.code === 'not_found'" variant="secondary" size="sm" @click="store.closeKey()">Close</Button>
          <Button v-else variant="secondary" size="sm" @click="store.reloadKey()">Try again</Button>
        </div>
      </div>

      <div v-else-if="!v" class="flex flex-1 flex-col gap-2 p-3" aria-busy="true" aria-label="Loading the value"><div v-for="i in 6" :key="i" class="h-6 animate-pulse rounded bg-surface" /></div>

      <template v-else>
        <ValueString v-if="v.type === 'string'" :value="v.value" :total-bytes="v.totalBytes" :truncated="v.truncated" :loading="store.valueLoading" :writable="writable" :save-label="saveLabel" @load-all="store.loadAll()" @save="saveString" />
        <template v-else-if="v.type === 'json'">
          <div class="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3"><Segmented v-model="jsonMode" label="Show as" :options="[{ value: 'tree', label: 'Tree' }, { value: 'text', label: 'Text' }]" class="w-auto" /></div>
          <ValueEditor v-if="jsonMode === 'text' && writable" :text="v.text" :save-label="saveLabel" json @save="saveJson" />
          <ValueJson v-else v-model:mode="jsonMode" :text="v.text" :value="jsonValue" />
        </template>
        <p v-else-if="v.type === 'unsupported'" class="p-4 text-sm text-muted">Dalan has no viewer for the type {{ v.keyType }} yet. The Console can still read it.</p>
        <p v-else-if="empty && !writable" class="p-4 text-sm text-muted">This {{ store.keyInfo?.key_type === 'zset' ? 'sorted set' : store.keyInfo?.key_type }} has no {{ unit?.[1] ?? 'items' }}.</p>
        <ValueHash v-else-if="v.type === 'hash'" :fields="v.fields" :tab-key="tabKey" :writable="writable" :save-label="saveLabel" />
        <ValueList v-else-if="v.type === 'list'" :items="v.items" :tab-key="tabKey" :writable="writable" :save-label="saveLabel" />
        <ValueSet v-else-if="v.type === 'set'" :members="v.members" :tab-key="tabKey" :writable="writable" :save-label="saveLabel" />
        <ValueZset v-else-if="v.type === 'zset'" :members="v.members" :tab-key="tabKey" :writable="writable" :save-label="saveLabel" />
        <ValueStream v-else-if="v.type === 'stream'" :entries="v.entries" :groups="store.keyInfo?.stream_groups ?? []" :tab-key="tabKey" :writable="writable" :save-label="saveLabel" />

        <div v-if="isCollection && !empty" class="flex h-9 shrink-0 items-center gap-2 border-t border-border px-3 text-xs text-muted" aria-live="polite">
          <span>{{ store.valueNext ? `Showing ${count(loaded)} of ${count(Math.max(total, loaded))}` : `All ${count(loaded)} ${loaded === 1 ? unit?.[0] : unit?.[1]} loaded` }}</span>
          <Button v-if="store.valueNext" variant="ghost" size="sm" class="text-accent" :loading="store.valueLoading" @click="store.loadMore()">Load more</Button>
        </div>
      </template>
    </section>

    <div v-else class="grid min-w-0 flex-1 place-items-center"><EmptyState :icon="Database" title="Pick a key to see its value." text="Keys are listed as they are scanned. Narrow them with a pattern such as user:*." /></div>

    <NewKeyDialog v-model:open="newKeyOpen" :tab-key="tabKey" :save-label="saveLabel" />
    <AlertDialog v-model:open="deleteAsk">
      <AlertDialogContent :data-testid="tid('redis-delete-open')">
        <AlertDialogHeader>
          <AlertDialogTitle>Delete {{ keyName }}?</AlertDialogTitle>
          <AlertDialogDescription>This database is Open, so the key is deleted as soon as you confirm. It can't be undone.</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <Button :class="buttonVariants({ variant: 'destructive' })" @click="deleteKey">Delete key</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
