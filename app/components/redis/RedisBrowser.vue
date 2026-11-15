<script setup lang="ts">
// S28: the Browser view of a Redis tab: the key tree on the left, the open key's value on the right.
// R2 is the read side only: nothing here can change data (the editors arrive with the write gate, R3).
import { CircleAlert, Database } from '@lucide/vue'
import { count, parsedJson, TYPE_UNIT } from '~/utils/redisBytes'

const props = defineProps<{ tabKey: string, separator: string }>()
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
</script>

<template>
  <div class="flex min-h-0 flex-1" :data-testid="tid('redis-browser')">
    <KeyTree :tab-key="tabKey" :separator="separator" />

    <section v-if="store.selectedKey" :aria-label="`Key ${store.keyInfo ? '' : 'loading'}`" class="flex min-w-0 flex-1 flex-col" :data-testid="tid('redis-value')">
      <KeyHeader v-if="store.keyInfo && store.keyInfo.key_type !== 'none'" :info="store.keyInfo" @reload="store.reloadKey()" />

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
        <ValueString v-if="v.type === 'string'" :value="v.value" :total-bytes="v.totalBytes" :truncated="v.truncated" :loading="store.valueLoading" @load-all="store.loadAll()" />
        <template v-else-if="v.type === 'json'">
          <div class="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3"><Segmented v-model="jsonMode" label="Show as" :options="[{ value: 'tree', label: 'Tree' }, { value: 'text', label: 'Text' }]" class="w-auto" /></div>
          <ValueJson v-model:mode="jsonMode" :text="v.text" :value="jsonValue" />
        </template>
        <p v-else-if="v.type === 'unsupported'" class="p-4 text-sm text-muted">Dalan has no viewer for the type {{ v.keyType }} yet. The Console can still read it.</p>
        <p v-else-if="empty" class="p-4 text-sm text-muted">This {{ store.keyInfo?.key_type === 'zset' ? 'sorted set' : store.keyInfo?.key_type }} has no {{ unit?.[1] ?? 'items' }}.</p>
        <ValueHash v-else-if="v.type === 'hash'" :fields="v.fields" />
        <ValueList v-else-if="v.type === 'list'" :items="v.items" />
        <ValueSet v-else-if="v.type === 'set'" :members="v.members" />
        <ValueZset v-else-if="v.type === 'zset'" :members="v.members" />
        <ValueStream v-else-if="v.type === 'stream'" :entries="v.entries" :groups="store.keyInfo?.stream_groups ?? []" />

        <div v-if="isCollection && !empty" class="flex h-9 shrink-0 items-center gap-2 border-t border-border px-3 text-xs text-muted" aria-live="polite">
          <span>{{ store.valueNext ? `Showing ${count(loaded)} of ${count(Math.max(total, loaded))}` : `All ${count(loaded)} ${loaded === 1 ? unit?.[0] : unit?.[1]} loaded` }}</span>
          <Button v-if="store.valueNext" variant="ghost" size="sm" class="text-accent" :loading="store.valueLoading" @click="store.loadMore()">Load more</Button>
        </div>
      </template>
    </section>

    <div v-else class="grid min-w-0 flex-1 place-items-center"><EmptyState :icon="Database" title="Pick a key to see its value." text="Keys are listed as they are scanned. Narrow them with a pattern such as user:*." /></div>
  </div>
</template>
