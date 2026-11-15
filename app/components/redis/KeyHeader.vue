<script setup lang="ts">
// S28, right side top: what the open key is. R2 only reads: rename, delete and the expiry editor arrive with the write gate.
import { Copy, RefreshCw } from '@lucide/vue'
import { toast } from 'vue-sonner'
import type { RedisKeyInfo_Serialize } from '~/types/bindings'
import { bytesText, count, sizeText, ttlText, TYPE_UNIT } from '~/utils/redisBytes'

const props = defineProps<{ info: RedisKeyInfo_Serialize }>()
defineEmits<{ reload: [] }>()

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
</script>

<template>
  <div class="flex h-11 shrink-0 items-center gap-2.5 border-b border-border px-3">
    <RedisTypeChip :type="info.key_type" />
    <h2 class="min-w-0 truncate font-mono text-sm font-semibold" :title="name">{{ name }}</h2>
    <IconButton label="Copy key name" @click="copyName"><Copy /></IconButton>
    <span class="shrink-0 text-xs text-muted">{{ expiry }}</span>
    <span class="shrink-0 text-xs text-muted">{{ size }}</span>
    <span class="ml-auto"><IconButton label="Reload value" @click="$emit('reload')"><RefreshCw /></IconButton></span>
  </div>
</template>
