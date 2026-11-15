<script setup lang="ts">
// S33 "Dangerous command": the person types the database's name before the button works. Rust compares the
// name again, so this dialog is a courtesy, not the check.
import { TriangleAlert } from '@lucide/vue'
import { Input } from '@/components/ui/input'
import { buttonVariants } from '@/components/ui/button'
import { count } from '~/utils/redisBytes'

const props = defineProps<{ tabKey: string, dbName: string }>()
const store = useRedisStore(props.tabKey)
const ask = computed(() => (store.pendingWrite?.pending.level === 'typed' ? store.pendingWrite : null))
const open = computed({ get: () => ask.value !== null, set: (v: boolean) => { if (!v) store.cancelWrite() } })
const typed = ref('')
watch(ask, () => { typed.value = '' })
const matches = computed(() => typed.value === props.dbName)
const lost = computed(() => {
  const n = ask.value?.pending.count
  return ask.value?.ask.text ?? (n ? `This removes the key and its ${count(n)} items. It can't be undone, and everyone using this database is affected.` : 'This can\'t be undone, and everyone using this database is affected.')
})
</script>

<template>
  <AlertDialog v-model:open="open">
    <AlertDialogContent v-if="ask" class="gap-3 border-danger sm:max-w-[440px]" :data-testid="tid('redis-danger')">
      <AlertDialogHeader>
        <AlertDialogTitle class="flex items-center gap-2"><TriangleAlert class="size-4 shrink-0 text-danger" aria-hidden="true" />{{ ask.ask.title.replace(/\?$/, '') }} on {{ dbName }}?</AlertDialogTitle>
        <AlertDialogDescription>{{ lost }}</AlertDialogDescription>
      </AlertDialogHeader>
      <pre class="max-h-32 overflow-auto whitespace-pre-wrap break-all rounded-md bg-bg px-2.5 py-2 font-mono text-xs" aria-label="The command" data-part="command">{{ ask.pending.command_preview }}</pre>
      <label class="flex flex-col gap-1 text-xs text-muted">Type {{ dbName }} to confirm
        <Input v-model="typed" class="font-mono text-xs text-text" autocomplete="off" spellcheck="false" @keydown.enter.prevent="matches && store.confirmWrite(typed)" />
      </label>
      <AlertDialogFooter>
        <AlertDialogCancel>Cancel</AlertDialogCancel>
        <Button :class="buttonVariants({ variant: 'destructive' })" :disabled="!matches" :loading="store.writeBusy" @click="store.confirmWrite(typed)">{{ ask.ask.action }}</Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
