<script setup lang="ts">
// S28: a new key with its first item. Rust never overwrites: a name that exists is refused. The change goes
// through the write gate like every other, so in Confirm mode it is reviewed before it is sent.
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { jsonProblem, parseExpiry, textBytes } from '~/utils/redisBytes'

const props = defineProps<{ tabKey: string, saveLabel: string }>()
const open = defineModel<boolean>('open', { required: true })
const store = useRedisStore(props.tabKey)

const TYPES = [
  { value: 'string', label: 'String', item: 'Value' }, { value: 'hash', label: 'Hash', item: 'Value' }, { value: 'list', label: 'List', item: 'First item' },
  { value: 'set', label: 'Set', item: 'First member' }, { value: 'zset', label: 'Sorted set', item: 'Member' }, { value: 'stream', label: 'Stream', item: 'Value' }, { value: 'json', label: 'JSON', item: 'JSON' },
] as const
const form = reactive({ name: '', type: 'string', field: '', value: '', score: '0', expiry: '' })
const error = ref('')
const busy = ref(false)
watch(open, (v) => {
  if (!v) return
  Object.assign(form, { name: store.pattern.endsWith('*') ? store.pattern.slice(0, -1) : '', type: 'string', field: '', value: '', score: '0', expiry: '' })
  error.value = ''
})
const kind = computed(() => TYPES.find(t => t.value === form.type)!)
const hasField = computed(() => form.type === 'hash' || form.type === 'stream')
const long = computed(() => form.type === 'string' || form.type === 'json')

async function submit() {
  error.value = ''
  if (!form.name) return void (error.value = 'A key needs a name.')
  if (hasField.value && !form.field) return void (error.value = 'Give the first field a name.')
  if (form.type === 'json' && jsonProblem(form.value)) return void (error.value = jsonProblem(form.value))
  if (form.type === 'zset' && !(/^[+-]?inf$/i.test(form.score.trim()) || (form.score.trim() !== '' && Number.isFinite(Number(form.score))))) return void (error.value = 'A score is a number, inf or -inf.')
  const ttl = form.expiry.trim() ? parseExpiry(form.expiry) : null
  if (form.expiry.trim() && ttl === null) return void (error.value = 'For the expiry, use 90s, 15m, 2h, 7d or a date in the future.')
  busy.value = true
  const ran = await store.write({
    op: 'create', key: textBytes(form.name), key_type: form.type, field: hasField.value ? textBytes(form.field) : null,
    value: textBytes(form.value), score: form.type === 'zset' ? form.score.trim() : null, ttl_ms: ttl,
  }, { title: 'Create this key?', action: 'Create key' })
  busy.value = false
  // It ran (Open mode), or it now waits in the confirmation: either way this form is done. A refusal keeps it open.
  if (ran || store.pendingWrite) open.value = false
}
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-[460px]" :data-testid="tid('redis-new-key')">
      <DialogHeader>
        <DialogTitle>New key</DialogTitle>
        <DialogDescription>A key starts with its first item. An existing key is never overwritten.</DialogDescription>
      </DialogHeader>
      <form class="flex flex-col gap-3" @submit.prevent="submit">
        <div class="flex flex-col gap-1.5"><Label for="rk-name">Name</Label><Input id="rk-name" v-model="form.name" class="font-mono text-xs" placeholder="user:1045:profile" autocomplete="off" spellcheck="false" /></div>
        <div class="flex flex-col gap-1.5">
          <Label id="rk-type-label">Type</Label>
          <Select v-model="form.type"><SelectTrigger aria-labelledby="rk-type-label"><SelectValue /></SelectTrigger><SelectContent><SelectItem v-for="t in TYPES" :key="t.value" :value="t.value">{{ t.label }}</SelectItem></SelectContent></Select>
        </div>
        <div v-if="hasField" class="flex flex-col gap-1.5"><Label for="rk-field">Field</Label><Input id="rk-field" v-model="form.field" class="font-mono text-xs" autocomplete="off" spellcheck="false" /></div>
        <div class="flex flex-col gap-1.5">
          <Label for="rk-value">{{ kind.item }}</Label>
          <Textarea v-if="long" id="rk-value" v-model="form.value" class="min-h-24 font-mono text-xs" spellcheck="false" />
          <Input v-else id="rk-value" v-model="form.value" class="font-mono text-xs" autocomplete="off" spellcheck="false" />
        </div>
        <div v-if="form.type === 'zset'" class="flex flex-col gap-1.5"><Label for="rk-score">Score</Label><Input id="rk-score" v-model="form.score" class="w-[140px] font-mono text-xs" autocomplete="off" /></div>
        <div class="flex flex-col gap-1.5"><Label for="rk-expiry">Expiry (optional)</Label><Input id="rk-expiry" v-model="form.expiry" class="w-[220px] text-xs" placeholder="90s, 15m, 2h, 7d or a date" autocomplete="off" spellcheck="false" /></div>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <DialogFooter>
          <Button variant="secondary" type="button" @click="open = false">Cancel</Button>
          <Button type="submit" :loading="busy">{{ saveLabel === 'Save' ? 'Create key' : saveLabel }}</Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>
</template>
