<script setup lang="ts">
// Several sessions dropped at once (the server restarted, the VPN fell over): one card instead of one message per tab.
import { Unplug } from '@lucide/vue'

const sessions = useSessionsStore()
const count = computed(() => sessions.droppedCard.length)
</script>

<template>
  <div v-if="count" role="status" class="fixed bottom-4 left-4 z-40 flex w-[300px] flex-col gap-2.5 rounded-lg border border-border bg-surface-raised p-3.5 shadow-pop">
    <div class="flex items-center gap-2 text-sm"><Unplug class="size-4 text-warning" aria-hidden="true" />{{ count }} connections dropped</div>
    <p class="truncate text-xs text-muted">{{ sessions.droppedCard.map(s => s.label).join(', ') }}</p>
    <div class="flex justify-end gap-2">
      <Button size="sm" variant="secondary" @click="sessions.dismissDropped()">Dismiss</Button>
      <Button size="sm" @click="sessions.reconnectAll()">Reconnect all</Button>
    </div>
  </div>
</template>
