<script setup lang="ts">
// S33: one action on every key that matches a pattern. Preview first (Rust walks the whole database with SCAN and
// keeps the matching keys), then type the database's name, then run in small batches. Stop keeps what is done.
// Rust refuses this in Read-only and checks the name itself; the button here is only the last step.
import { Search } from '@lucide/vue'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { buttonVariants } from '@/components/ui/button'
import type { RedisBulkAction, RedisBulkPreview, RedisBulkResult } from '~/types/bindings'
import { bytesText, count, parseExpiry } from '~/utils/redisBytes'

const props = defineProps<{ tabKey: string, dbName: string }>()
const open = defineModel<boolean>('open', { required: true })
const store = useRedisStore(props.tabKey)

const TYPES = [{ value: 'all', label: 'All types' }, { value: 'string', label: 'String' }, { value: 'hash', label: 'Hash' }, { value: 'list', label: 'List' }, { value: 'set', label: 'Set' }, { value: 'zset', label: 'Sorted set' }, { value: 'stream', label: 'Stream' }, { value: 'json', label: 'JSON' }]
const pattern = ref('')
const type = ref('all')
const action = ref('delete')
const expiry = ref('')
const typed = ref('')
const error = ref('')
const phase = ref<'idle' | 'scanning' | 'ready' | 'running' | 'done'>('idle')
const found = ref(0)
const preview = ref<RedisBulkPreview | null>(null)
const progress = ref({ done: 0, total: 0 })
const result = ref<RedisBulkResult | null>(null)

function reset() {
  phase.value = 'idle'
  preview.value = null
  result.value = null
  found.value = 0
  typed.value = ''
  error.value = ''
}
watch(open, (v) => {
  if (!v) return void store.bulkStop()
  pattern.value = store.pattern
  type.value = store.typeFilter ?? 'all'
  action.value = 'delete'
  expiry.value = ''
  reset()
})
// Another pattern or type is another set of keys: the old preview says nothing about it.
watch([pattern, type], () => { if (phase.value === 'ready' || phase.value === 'done') reset() })

async function runPreview() {
  reset()
  phase.value = 'scanning'
  try {
    const p = await store.bulkPreview(pattern.value.trim(), type.value === 'all' ? null : type.value, (e) => { if (e.type === 'scanning') found.value = e.found ?? 0 })
    preview.value = p
    phase.value = 'ready'
  }
  catch (e) {
    error.value = e instanceof Error ? e.message : 'Couldn\'t scan the database.'
    phase.value = 'idle'
  }
}

const total = computed(() => preview.value?.count ?? 0)
const ttl = computed(() => (action.value === 'expire' ? parseExpiry(expiry.value) : null))
const verb = computed(() => ({ delete: ['Delete', 'Deleting', 'Deleted'], expire: ['Set expiry on', 'Setting the expiry on', 'Set the expiry on'], persist: ['Remove expiry from', 'Removing the expiry from', 'Removed the expiry from'] }[action.value as 'delete' | 'expire' | 'persist']))
const keysWord = (n: number) => `${count(n)} ${n === 1 ? 'key' : 'keys'}`
const canRun = computed(() => phase.value === 'ready' && !!preview.value?.id && typed.value === props.dbName && (action.value !== 'expire' || ttl.value !== null))

async function run() {
  const p = preview.value
  if (!canRun.value || !p) return
  const what: RedisBulkAction = action.value === 'expire' ? { action: 'expire', ttl_ms: ttl.value! } : action.value === 'persist' ? { action: 'persist' } : { action: 'delete' }
  progress.value = { done: 0, total: p.count ?? 0 }
  phase.value = 'running'
  error.value = ''
  try {
    result.value = await store.bulkRun(p.id, what, typed.value, (e) => { if (e.type === 'running') progress.value = { done: e.done ?? 0, total: e.total ?? 0 } })
    phase.value = 'done'
  }
  catch (e) {
    error.value = e instanceof Error ? e.message : 'The action failed.'
    reset()
    error.value = e instanceof Error ? e.message : 'The action failed.'
  }
}
const sample = computed(() => (preview.value?.sample ?? []).map(bytesText))
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="gap-3 sm:max-w-[520px]" :data-testid="tid('redis-bulk')" @open-auto-focus.prevent="($event.target as HTMLElement)?.querySelector<HTMLElement>('[data-cancel]')?.focus()">
      <DialogHeader>
        <DialogTitle>Actions on matching keys</DialogTitle>
        <DialogDescription>Preview what matches, then run. Stop keeps what's already done.</DialogDescription>
      </DialogHeader>

      <form class="flex gap-2" @submit.prevent="runPreview">
        <label class="relative min-w-0 flex-1"><Search class="pointer-events-none absolute left-2 top-2 size-4 text-muted" aria-hidden="true" /><Input v-model="pattern" aria-label="Key pattern" class="pl-8 font-mono text-xs" placeholder="session:*" autocomplete="off" spellcheck="false" :disabled="phase === 'scanning' || phase === 'running'" /></label>
        <Select v-model="type" :disabled="phase === 'scanning' || phase === 'running'"><SelectTrigger class="w-auto gap-1" aria-label="Type"><SelectValue /></SelectTrigger><SelectContent><SelectItem v-for="t in TYPES" :key="t.value" :value="t.value">{{ t.label }}</SelectItem></SelectContent></Select>
        <Button variant="secondary" type="submit" :disabled="phase === 'scanning' || phase === 'running'">Preview</Button>
      </form>

      <div class="flex flex-wrap items-center gap-2">
        <Segmented v-model="action" label="Action" :options="[{ value: 'delete', label: 'Delete' }, { value: 'expire', label: 'Set expiry' }, { value: 'persist', label: 'Remove expiry' }]" class="w-auto [&_button]:whitespace-nowrap" />
        <Input v-if="action === 'expire'" v-model="expiry" aria-label="Expires in" class="h-7 w-[200px] text-xs" placeholder="90s, 15m, 2h, 7d or a date" autocomplete="off" spellcheck="false" :aria-invalid="(!!expiry && ttl === null) || undefined" />
      </div>

      <div v-if="phase === 'scanning'" class="flex items-center gap-2 rounded-lg border border-border bg-bg px-2.5 py-2 text-sm" role="status">
        <span class="size-3 animate-spin rounded-full border-2 border-accent border-r-transparent" aria-hidden="true" />Scanning… {{ count(found) }} found
        <Button variant="secondary" size="sm" class="ml-auto" @click="store.bulkStop()">Stop</Button>
      </div>
      <div v-else-if="preview" class="rounded-lg border border-border bg-bg" role="status" :data-testid="tid('bulk-preview')">
        <p class="flex items-center gap-2 px-2.5 py-2"><b class="font-semibold">{{ total ? `${keysWord(total)} ${total === 1 ? 'matches' : 'match'}` : `No keys match ${pattern || '*'}.` }}</b><span class="text-xs text-muted">{{ preview.complete ? 'whole database scanned' : 'Stopped before the whole database was scanned. Preview again to run.' }}</span></p>
        <p v-if="sample.length" class="break-all px-2.5 pb-2 font-mono text-xs leading-5 text-muted">{{ sample.join(' · ') }}<template v-if="total > sample.length"> … {{ count(total - sample.length) }} more</template></p>
      </div>

      <div v-if="phase === 'running'" role="status">
        <div class="h-1.5 rounded-full bg-surface-hover" role="progressbar" :aria-valuenow="progress.done" :aria-valuemax="progress.total" aria-label="Progress"><div class="h-1.5 rounded-full bg-accent transition-[width]" :style="{ width: `${progress.total ? (progress.done / progress.total) * 100 : 0}%` }" /></div>
        <p class="mt-1 text-xs text-muted">{{ verb[1] }} {{ count(progress.done) }} of {{ count(progress.total) }} · in small batches, so the server stays responsive</p>
      </div>
      <p v-else-if="phase === 'done' && result" role="status" class="text-sm" :data-testid="tid('bulk-result')">
        {{ result.stopped ? `Stopped. ${count(result.processed ?? 0)} of ${count(total)} done; the rest were left alone.` : `${verb[2]} ${keysWord(result.changed ?? 0)}.` }}
        <span v-if="!result.stopped && (result.changed ?? 0) < (result.processed ?? 0)" class="text-muted">{{ count((result.processed ?? 0) - (result.changed ?? 0)) }} were already gone or unchanged.</span>
      </p>

      <label v-if="phase === 'ready' && preview?.id" class="flex flex-col gap-1 text-xs text-muted">Type {{ dbName }} to confirm
        <Input v-model="typed" class="font-mono text-xs text-text" autocomplete="off" spellcheck="false" />
      </label>
      <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>

      <DialogFooter>
        <Button v-if="phase === 'running'" variant="secondary" @click="store.bulkStop()">Stop</Button>
        <template v-else>
          <Button variant="secondary" data-cancel @click="open = false">{{ phase === 'done' ? 'Close' : 'Cancel' }}</Button>
          <Button v-if="phase !== 'done'" :class="buttonVariants({ variant: 'destructive' })" :disabled="!canRun" @click="run">{{ verb[0] }} {{ total ? keysWord(total) : 'keys' }}</Button>
        </template>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
