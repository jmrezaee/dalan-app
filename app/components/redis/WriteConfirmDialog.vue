<script setup lang="ts">
// S33 "Confirm a write": what the gate in Rust says a change will do (now, after, the exact command), before it is
// sent. Cancel has the focus. The button only tells Rust to run what it already holds.
import { buttonVariants } from '@/components/ui/button'
import { bytesText, count } from '~/utils/redisBytes'

const props = defineProps<{ tabKey: string, dbName: string, production: boolean }>()
const store = useRedisStore(props.tabKey)
const ask = computed(() => (store.pendingWrite?.pending.level === 'confirm' ? store.pendingWrite : null))
const open = computed({ get: () => ask.value !== null, set: (v: boolean) => { if (!v) store.cancelWrite() } })
const p = computed(() => ask.value?.pending)
</script>

<template>
  <AlertDialog v-model:open="open">
    <AlertDialogContent v-if="ask && p" class="gap-3 sm:max-w-[440px]" :data-testid="tid('redis-confirm')">
      <AlertDialogHeader>
        <AlertDialogTitle>{{ ask.ask.title }}</AlertDialogTitle>
        <AlertDialogDescription class="sr-only">Nothing is sent until you confirm.</AlertDialogDescription>
      </AlertDialogHeader>
      <dl class="grid grid-cols-[64px_minmax(0,1fr)] gap-x-2.5 gap-y-1 text-xs">
        <dt class="text-muted">Database</dt><dd>{{ dbName }} <span v-if="production" class="text-warning">· production</span></dd>
        <dt class="text-muted">Target</dt><dd class="break-all font-mono">{{ p.target }}</dd>
        <template v-if="p.before"><dt class="text-muted">Now</dt><dd class="max-h-24 overflow-auto break-all font-mono text-danger" data-part="now">{{ bytesText(p.before) }}</dd></template>
        <template v-if="p.after"><dt class="text-muted">After</dt><dd class="max-h-24 overflow-auto break-all font-mono text-success" data-part="after">{{ bytesText(p.after) }}</dd></template>
        <template v-if="p.count !== null"><dt class="text-muted">Items</dt><dd>{{ count(p.count ?? 0) }} go with it</dd></template>
      </dl>
      <pre class="max-h-32 overflow-auto whitespace-pre-wrap break-all rounded-md bg-bg px-2.5 py-2 font-mono text-xs" aria-label="The command" data-part="command">{{ p.command_preview }}</pre>
      <AlertDialogFooter>
        <AlertDialogCancel>Cancel</AlertDialogCancel>
        <Button :class="ask.ask.danger ? buttonVariants({ variant: 'destructive' }) : ''" :loading="store.writeBusy" @click="store.confirmWrite()">{{ ask.ask.action }}</Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
