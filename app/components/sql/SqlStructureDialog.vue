<script setup lang="ts">
// S42 "Review a structure change" (F24): the statements Rust generated and holds, what each loses or locks, and the
// database's name when something is dropped or rewritten. They run in one transaction.
import { TriangleAlert } from '@lucide/vue'
import { Input } from '@/components/ui/input'
import { buttonVariants } from '@/components/ui/button'
import type { TableTab } from '~/stores/sql'

const props = defineProps<{ tabKey: string, tab: TableTab }>()
const store = useSqlStore(props.tabKey)
const r = computed(() => props.tab.structureReview)
const open = computed({ get: () => r.value !== null, set: (v: boolean) => { if (!v) void store.cancelStructure(props.tab) } })
const typed = ref('')
watch(r, () => { typed.value = '' })
const typedLevel = computed(() => r.value?.pending.level === 'typed')
const ok = computed(() => !typedLevel.value || typed.value === r.value?.pending.db_name)
</script>

<template>
  <AlertDialog v-model:open="open">
    <AlertDialogContent v-if="r" :class="['gap-3 sm:max-w-[560px]', typedLevel && 'border-danger']" :data-testid="tid('structure-review')">
      <AlertDialogHeader>
        <AlertDialogTitle class="flex items-center gap-2"><TriangleAlert v-if="typedLevel" class="size-4 shrink-0 text-danger" aria-hidden="true" />Change {{ tab.table }} on {{ r.pending.db_name }}?</AlertDialogTitle>
        <AlertDialogDescription>These run together in one transaction: if one fails, none is kept. <span v-if="r.pending.environment === 'production'" class="text-warning">This is a production database.</span></AlertDialogDescription>
      </AlertDialogHeader>
      <ol class="flex max-h-56 list-none flex-col gap-2 overflow-auto p-0">
        <li v-for="(x, i) in r.pending.statements" :key="i"><pre class="whitespace-pre-wrap break-all rounded-md border border-border bg-bg p-2 font-mono text-xs" aria-label="Statement">{{ x }};</pre></li>
      </ol>
      <ul v-if="r.pending.warnings.length" class="m-0 flex list-none flex-col gap-1 p-0 text-xs text-warning" data-part="warnings"><li v-for="w in r.pending.warnings" :key="w">{{ w }}</li></ul>
      <label v-if="typedLevel" class="flex flex-col gap-1 text-xs text-muted">Type {{ r.pending.db_name }} to confirm
        <Input v-model="typed" class="font-mono text-xs text-text" autocomplete="off" spellcheck="false" @keydown.enter.prevent="ok && store.applyStructure(tab, typed)" />
      </label>
      <p v-if="r.error" role="alert" class="text-sm text-danger" data-part="error">{{ r.error }}</p>
      <AlertDialogFooter>
        <AlertDialogCancel>{{ r.error ? 'Close' : 'Cancel' }}</AlertDialogCancel>
        <Button :class="typedLevel ? buttonVariants({ variant: 'destructive' }) : ''" :disabled="!ok" :loading="r.busy" @click="store.applyStructure(tab, typedLevel ? typed : null)">Apply</Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
