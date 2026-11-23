<script setup lang="ts">
// S42 "Confirm a change" and "Dangerous statement" for the query editor: what the gate in Rust holds, with what the
// server says each statement would do. The dangerous level asks for the database's name. Cancel has the focus.
// Rust asks the mode again and compares the name, so this dialog is a courtesy, not the check.
import { TriangleAlert } from '@lucide/vue'
import { Input } from '@/components/ui/input'
import { buttonVariants } from '@/components/ui/button'
import type { QueryTab } from '~/stores/sql'

const props = defineProps<{ tabKey: string, query: QueryTab }>()
const store = useSqlStore(props.tabKey)
const p = computed(() => props.query.pending)
const open = computed({ get: () => p.value !== null, set: (v: boolean) => { if (!v) void store.discardRun(props.query) } })
const typed = ref('')
watch(p, () => { typed.value = '' })
const typedLevel = computed(() => p.value?.level === 'typed')
const matches = computed(() => !typedLevel.value || typed.value === p.value?.db_name)
const CLASS_TEXT = { read: 'Read', write: 'Changes data', schema: 'Changes structure', dangerous: 'Dangerous' } as const
</script>

<template>
  <AlertDialog v-model:open="open">
    <AlertDialogContent v-if="p" :class="['gap-3 sm:max-w-[520px]', typedLevel && 'border-danger']" :data-testid="tid(typedLevel ? 'sql-danger' : 'sql-confirm')">
      <AlertDialogHeader>
        <AlertDialogTitle class="flex items-center gap-2">
          <TriangleAlert v-if="typedLevel" class="size-4 shrink-0 text-danger" aria-hidden="true" />{{ typedLevel ? `Dangerous statement on ${p.db_name}?` : `Run ${p.statements.length === 1 ? 'this change' : `these ${p.statements.length} statements`}?` }}
        </AlertDialogTitle>
        <AlertDialogDescription>
          <template v-if="typedLevel">This can't be undone, and everyone using this database is affected.</template>
          <template v-else>Nothing runs until you confirm. <span v-if="p.environment === 'production'" class="text-warning">{{ p.db_name }} is a production database.</span></template>
        </AlertDialogDescription>
      </AlertDialogHeader>
      <ol class="flex max-h-64 list-none flex-col gap-2 overflow-auto p-0">
        <li v-for="s in p.statements" :key="s.index" class="rounded-md border border-border bg-bg p-2">
          <div class="mb-1 flex items-center gap-2 text-2xs"><span :class="s.class === 'dangerous' ? 'text-danger' : s.class === 'read' ? 'text-muted' : 'text-warning'">{{ CLASS_TEXT[s.class] }}</span></div>
          <pre class="max-h-24 overflow-auto whitespace-pre-wrap break-all font-mono text-xs" aria-label="Statement">{{ s.text }}</pre>
          <p v-if="s.note" class="mt-1 text-xs text-muted" data-part="note">{{ s.note }}</p>
        </li>
      </ol>
      <label v-if="typedLevel" class="flex flex-col gap-1 text-xs text-muted">Type {{ p.db_name }} to confirm
        <Input v-model="typed" class="font-mono text-xs text-text" autocomplete="off" spellcheck="false" @keydown.enter.prevent="matches && store.confirmRun(query, typed)" />
      </label>
      <AlertDialogFooter>
        <AlertDialogCancel>Cancel</AlertDialogCancel>
        <Button :class="typedLevel ? buttonVariants({ variant: 'destructive' }) : ''" :disabled="!matches" @click="store.confirmRun(query, typedLevel ? typed : null)">{{ typedLevel ? 'Run it anyway' : 'Run' }}</Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
