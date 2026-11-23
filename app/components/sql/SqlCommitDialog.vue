<script setup lang="ts">
// S42 "Commit review" for grid edits (F22): the exact statements Rust built, shown before anything is written. They
// run in one transaction; if a row was changed by someone else since it was loaded, nothing is written and this
// dialog says so (the optimistic check is in the statements themselves).
import { Button } from '@/components/ui/button'
import type { TableTab } from '~/stores/sql'

const props = defineProps<{ tabKey: string, tab: TableTab }>()
const store = useSqlStore(props.tabKey)
const r = computed(() => props.tab.review)
const open = computed({ get: () => r.value !== null, set: (v: boolean) => { if (!v) void store.cancelReview(props.tab) } })
</script>

<template>
  <AlertDialog v-model:open="open">
    <AlertDialogContent v-if="r" class="gap-3 sm:max-w-[560px]" :data-testid="tid('sql-commit')">
      <AlertDialogHeader>
        <AlertDialogTitle>Commit {{ r.pending.statements.length }} {{ r.pending.statements.length === 1 ? 'change' : 'changes' }} to {{ r.pending.db_name }}?</AlertDialogTitle>
        <AlertDialogDescription>
          They run together in one transaction: if any row was changed by someone else since you loaded it, nothing is written.
          <span v-if="r.pending.environment === 'production'" class="text-warning">This is a production database.</span>
        </AlertDialogDescription>
      </AlertDialogHeader>
      <ol class="flex max-h-64 list-none flex-col gap-2 overflow-auto p-0">
        <li v-for="(s, i) in r.pending.statements" :key="i"><pre class="whitespace-pre-wrap break-all rounded-md border border-border bg-bg p-2 font-mono text-xs" aria-label="Statement">{{ s }}</pre></li>
      </ol>
      <p v-if="r.error" role="alert" class="text-sm text-danger" data-part="error">{{ r.error }}</p>
      <AlertDialogFooter>
        <AlertDialogCancel>{{ r.error ? 'Close' : 'Cancel' }}</AlertDialogCancel>
        <Button :loading="r.busy" @click="store.commitReview(tab)">Commit</Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
