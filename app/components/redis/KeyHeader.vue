<script setup lang="ts">
// S28, right side top: what the open key is, and (R3, when the database isn't Read-only) its expiry control,
// Rename and Delete key. The header only says what the person wants; the browser sends it through the write gate.
import { Clock, Copy, RefreshCw } from '@lucide/vue'
import { toast } from 'vue-sonner'
import { Input } from '@/components/ui/input'
import type { RedisKeyInfo_Serialize } from '~/types/bindings'
import { bytesText, count, parseExpiry, sizeText, ttlText, TYPE_UNIT } from '~/utils/redisBytes'

const props = defineProps<{ info: RedisKeyInfo_Serialize, writable: boolean, saveLabel: string }>()
const emit = defineEmits<{ reload: [], rename: [to: string], expire: [ttlMs: number | null], delete: [] }>()

const name = computed(() => bytesText(props.info.key))
const expiry = computed(() => (props.info.ttl_ms === null ? 'No expiry' : `Expires in ${ttlText(props.info.ttl_ms)}`))
const size = computed(() => {
  const n = props.info.length ?? 0
  const memory = props.info.memory_bytes === null ? '' : ` · ${sizeText(props.info.memory_bytes)}`
  if (props.info.key_type === 'string') return `${count(n)} ${n === 1 ? 'byte' : 'bytes'}`
  const unit = TYPE_UNIT[props.info.key_type]
  return unit ? `${count(n)} ${n === 1 ? unit[0] : unit[1]}${memory}` : memory.replace(' · ', '')
})
async function copyName() {
  try {
    await navigator.clipboard.writeText(name.value)
    toast('Key name copied.')
  }
  catch {
    toast.error('Couldn\'t copy the key name.')
  }
}

// One inline form at a time: the expiry or the name.
const mode = ref<'' | 'expiry' | 'rename'>('')
const draft = ref('')
const problem = ref('')
const field = ref<{ $el: HTMLElement } | null>(null)
function start(m: 'expiry' | 'rename') {
  mode.value = m
  draft.value = m === 'rename' ? name.value : ''
  problem.value = ''
  nextTick(() => field.value?.$el.focus())
}
const stop = () => { mode.value = '' }
// Another key, or this one read again after a change: the form is over.
watch(() => props.info, stop)
function submit() {
  if (mode.value === 'expiry') {
    const ms = parseExpiry(draft.value)
    if (ms === null) return void (problem.value = 'Use 90s, 15m, 2h, 7d or a date in the future.')
    emit('expire', ms)
  }
  else if (mode.value === 'rename') {
    const to = draft.value
    if (!to) return void (problem.value = 'A key needs a name.')
    if (to === name.value) return stop()
    emit('rename', to)
  }
}
</script>

<template>
  <div class="flex min-h-11 shrink-0 flex-wrap items-center gap-x-2.5 gap-y-1 border-b border-border px-3 py-1">
    <RedisTypeChip :type="info.key_type" />

    <form v-if="mode === 'rename'" class="flex min-w-0 flex-1 items-center gap-1.5" @submit.prevent="submit" @keydown.esc.prevent="stop">
      <Input ref="field" v-model="draft" aria-label="New name" class="h-7 min-w-0 flex-1 font-mono text-xs" autocomplete="off" spellcheck="false" :aria-invalid="!!problem || undefined" />
      <span v-if="problem" role="alert" class="text-xs text-danger">{{ problem }}</span>
      <Button variant="secondary" size="sm" type="button" @click="stop">Cancel</Button>
      <Button size="sm" type="submit">Rename</Button>
    </form>
    <template v-else>
      <h2 class="min-w-0 truncate font-mono text-sm font-semibold" :title="name">{{ name }}</h2>
      <IconButton label="Copy key name" @click="copyName"><Copy /></IconButton>
    </template>

    <form v-if="mode === 'expiry'" class="flex items-center gap-1.5" @submit.prevent="submit" @keydown.esc.prevent="stop">
      <Input ref="field" v-model="draft" aria-label="Expires in" class="h-7 w-[190px] text-xs" placeholder="90s, 15m, 2h, 7d or a date" autocomplete="off" spellcheck="false" :aria-invalid="!!problem || undefined" />
      <Button size="sm" type="submit">Set expiry</Button>
      <Button v-if="info.ttl_ms !== null" variant="secondary" size="sm" type="button" @click="emit('expire', null)">Remove expiry</Button>
      <Button variant="secondary" size="sm" type="button" @click="stop">Cancel</Button>
      <span v-if="problem" role="alert" class="text-xs text-danger">{{ problem }}</span>
    </form>
    <template v-else-if="mode === ''">
      <Button v-if="writable" variant="ghost" size="sm" class="shrink-0 px-1.5 text-xs text-muted" :title="'Change the expiry'" @click="start('expiry')"><Clock />{{ expiry }}</Button>
      <span v-else class="shrink-0 text-xs text-muted">{{ expiry }}</span>
      <span class="shrink-0 text-xs text-muted">{{ size }}</span>
    </template>

    <span v-if="mode === ''" class="ml-auto flex items-center gap-1">
      <template v-if="writable">
        <Button v-if="info.key.kind === 'text'" variant="ghost" size="sm" @click="start('rename')">Rename</Button>
        <Button variant="ghost" size="sm" class="text-danger" @click="emit('delete')">Delete key</Button>
      </template>
      <IconButton label="Reload value" @click="$emit('reload')"><RefreshCw /></IconButton>
    </span>
  </div>
</template>
