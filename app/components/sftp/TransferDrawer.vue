<script setup lang="ts">
// S22: the queue. Each row is one file or folder being copied; a conflict shows its three choices inline.
import { ChevronDown, ChevronUp, TriangleAlert } from '@lucide/vue'
import { Checkbox } from '@/components/ui/checkbox'
import { formatBytes, formatModified, formatSpeed } from '~/utils/format'
import type { TransferItemRow, TransferRow } from '~/stores/sftp'

const sftp = useSftpStore()
const collapsed = ref(false)
const applyAll = ref(false)

const items = computed(() => sftp.transfers.flatMap(t => t.items.map(i => ({ t, i }))))
const total = computed(() => items.value.reduce((n, x) => n + x.i.total, 0))
const done = computed(() => items.value.reduce((n, x) => n + Math.min(x.i.bytes, x.i.total), 0))
const percentAll = computed(() => (total.value ? Math.round((done.value / total.value) * 100) : 0))
const percent = (i: TransferItemRow) => (i.status === 'done' ? 100 : i.total ? Math.min(100, Math.round((i.bytes / i.total) * 100)) : 0)
function speed(i: TransferItemRow) {
  if (i.status !== 'running' || !i.startedAt) return ''
  const secs = (Date.now() - i.startedAt) / 1000
  return secs > 0.3 ? `${formatBytes(i.bytes)} of ${formatBytes(i.total)} · ${formatSpeed(i.bytes / secs)}` : ''
}
const status = (i: TransferItemRow) => (i.status === 'failed' ? 'failed' : i.status === 'done' ? 'done' : 'running')
const direction = (i: TransferItemRow) => (i.direction === 'down' ? 'down' : 'up')
function resolve(t: TransferRow, choice: 'replace' | 'keep_both' | 'skip') {
  sftp.resolveConflict(t, choice, applyAll.value)
  applyAll.value = false
}
const hasFinished = computed(() => sftp.transfers.some(t => t.finished))
</script>

<template>
  <section v-if="sftp.transfers.length" aria-label="Transfers" class="flex shrink-0 flex-col border-t border-border bg-surface" :class="collapsed ? '' : 'max-h-56'">
    <div class="flex h-8 shrink-0 items-center gap-2.5 border-b border-border px-3">
      <span class="font-medium">Transfers</span>
      <span class="text-xs text-muted">{{ items.length }} {{ items.length === 1 ? 'item' : 'items' }}<template v-if="total"> · {{ percentAll }}%</template></span>
      <Button v-if="hasFinished" variant="ghost" size="sm" class="ml-auto text-muted" @click="sftp.clearFinished()">Clear finished</Button>
      <IconButton :label="collapsed ? 'Expand transfers' : 'Collapse transfers'" size="xs" :class="hasFinished ? '' : 'ml-auto'" @click="collapsed = !collapsed"><ChevronUp v-if="collapsed" /><ChevronDown v-else /></IconButton>
    </div>
    <div v-if="!collapsed" class="flex flex-col gap-1 overflow-auto px-3 py-1.5">
      <template v-for="t in sftp.transfers" :key="t.id || t.items[0]?.name">
        <template v-for="i in t.items" :key="i.index">
          <div v-if="t.conflict && t.conflict.item === i.index" class="-mx-2 flex items-center gap-2.5 rounded-md bg-warning/10 px-2 py-2">
            <TriangleAlert class="size-4 shrink-0 text-warning" aria-hidden="true" />
            <span class="min-w-0 flex-1">
              <span class="truncate">{{ t.conflict.name }} already exists there</span>
              <span class="block text-xs text-muted">yours {{ formatBytes(t.conflict.srcSize) }}, {{ formatModified(t.conflict.srcMtime) }} · theirs {{ formatBytes(t.conflict.destSize) }}, {{ formatModified(t.conflict.destMtime) }}</span>
            </span>
            <label class="flex items-center gap-1.5 text-xs text-muted"><Checkbox v-model="applyAll" />Apply to all</label>
            <Button size="sm" variant="secondary" @click="resolve(t, 'skip')">Skip</Button>
            <Button size="sm" variant="secondary" @click="resolve(t, 'keep_both')">Keep both</Button>
            <Button size="sm" @click="resolve(t, 'replace')">Replace</Button>
          </div>
          <div v-else>
            <ProgressRow :data-testid="tid(`transfer-row-${t.id}-${i.name}`)" :name="i.name" :direction="direction(i)" :percent="percent(i)" :speed="speed(i)" :status="status(i)" @cancel="sftp.cancel(t)" />
            <p v-if="i.status === 'failed'" role="alert" class="pl-7 text-xs text-danger">{{ i.message }}</p>
          </div>
        </template>
      </template>
    </div>
  </section>
</template>
