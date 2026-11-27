<script setup lang="ts">
// MongoDB import (D17, E6): JSON lines, a JSON array, extended JSON, or CSV whose header may type its columns
// (`age.int32()`), into a collection: insert, upsert by a field, or replace what is there (typed). Rust reads the
// file and writes the documents in batches; a document the server refuses doesn't stop the others.
import { FileJson } from '@lucide/vue'
import { Button, buttonVariants } from '@/components/ui/button'
import { Input } from '@/components/ui/input'

const props = defineProps<{ tabKey: string, connId: string, dbs: string[] }>()
const bulk = useBulkStore(props.tabKey)
const mg = computed(() => bulk.mg)
watch(() => props.dbs, (d) => { if (!d.includes(bulk.mg.db)) bulk.mg.db = d[0] ?? '' }, { immediate: true })
const fmt = (n: number | null | undefined) => Math.round(n ?? 0).toLocaleString('en-US')
const FORMAT = { json_lines: 'JSON lines', json_array: 'a JSON array', csv: 'CSV' } as const
const typedOk = computed(() => !mg.value.pending || mg.value.pending.level !== 'typed' || mg.value.typed === mg.value.pending.typed_name)
</script>

<template>
  <section class="flex flex-col gap-3 rounded-lg border border-border bg-surface p-4 text-sm" aria-label="Import into a collection" :data-testid="tid('mongo-import')">
    <h2 class="text-sm font-semibold">Import into a collection</h2>
    <div class="flex items-center gap-2"><Button size="sm" variant="secondary" @click="bulk.mgPick(connId)"><FileJson class="size-3.5" aria-hidden="true" />Choose a file…</Button><span class="truncate font-mono text-xs">{{ mg.name }}</span></div>
    <p class="text-xs text-muted">JSON lines, a JSON array or extended JSON (<code>{"$oid": …}</code>, <code>{"$date": …}</code>), or CSV whose header can type a column: <code>age.int32()</code>, <code>born.date()</code>, <code>id.objectId()</code>; <code>addr.city</code> makes a nested field.</p>
    <template v-if="mg.preview">
      <p class="text-xs" data-part="mg-preview">{{ fmt(mg.preview.docs) }} documents in {{ FORMAT[mg.preview.format] }}.</p>
      <pre class="max-h-40 overflow-auto rounded-md border border-border bg-bg p-2 font-mono text-xs" aria-label="First documents">{{ mg.preview.sample.join('\n') }}</pre>
      <div class="flex flex-wrap items-center gap-2 text-xs">
        <select v-model="bulk.mg.db" class="h-8 rounded-md border border-border-input bg-bg px-2" aria-label="Database to import into">
          <option v-for="d in dbs" :key="d" :value="d">{{ d }}</option>
        </select>
        <Input v-model="bulk.mg.coll" class="h-8 w-48 font-mono text-xs" aria-label="Collection to import into" placeholder="collection" />
      </div>
      <div role="radiogroup" aria-label="How the documents go in" class="flex flex-col gap-1 text-xs">
        <label class="flex items-center gap-2"><input v-model="bulk.mg.mode" type="radio" name="mg-mode" value="insert" class="accent-primary">Insert (a document whose _id is there already is refused)</label>
        <label class="flex items-center gap-2"><input v-model="bulk.mg.mode" type="radio" name="mg-mode" value="upsert" class="accent-primary">Upsert by
          <select v-model="bulk.mg.field" class="h-7 rounded-md border border-border-input bg-bg px-1" aria-label="Field to upsert by" :disabled="mg.mode !== 'upsert'">
            <option v-for="f in mg.preview.fields.includes('_id') ? mg.preview.fields : ['_id', ...mg.preview.fields]" :key="f" :value="f">{{ f }}</option>
          </select>
        </label>
        <label class="flex items-center gap-2"><input v-model="bulk.mg.mode" type="radio" name="mg-mode" value="replace" class="accent-primary"><span>Replace the collection: <span class="text-danger">every document there is removed first</span></span></label>
      </div>
      <div v-if="!mg.pending" class="flex justify-end"><Button :disabled="!mg.coll.trim() || !mg.db" :data-testid="tid('mongo-import-review')" @click="bulk.mgPrepare(connId)">Review the import</Button></div>
    </template>
    <p v-if="mg.error" role="alert" class="text-xs text-danger">{{ mg.error }}</p>
    <section v-if="mg.pending" class="flex flex-col gap-2 rounded-md border border-border bg-bg p-3 text-xs" aria-label="Review the MongoDB import" :data-testid="tid('mongo-import-plan')">
      <p class="font-medium">{{ mg.mode === 'replace' ? 'Replace every document of' : mg.mode === 'upsert' ? `Upsert by ${mg.field} into` : 'Insert into' }} {{ mg.db }}.{{ mg.coll }}: {{ fmt(mg.pending.docs) }} documents?</p>
      <p v-if="mg.mode === 'replace'" class="text-danger">The documents there are removed first; this can't be undone (the collection's indexes stay).</p>
      <label v-if="mg.pending.level === 'typed'" class="flex flex-col gap-1">Type <span class="font-mono">{{ mg.pending.typed_name }}</span> to run it<Input v-model="bulk.mg.typed" class="h-8 w-72 font-mono" :aria-label="`Type ${mg.pending.typed_name} to confirm`" /></label>
      <div class="flex justify-end gap-2"><Button size="sm" variant="secondary" @click="bulk.mgCancel(connId)">Cancel</Button><Button size="sm" :class="mg.pending.level === 'typed' ? buttonVariants({ variant: 'destructive' }) : undefined" :disabled="!typedOk" :loading="mg.busy" :data-testid="tid('mongo-import-commit')" @click="bulk.mgCommit(connId)">Import</Button></div>
    </section>
    <p v-if="mg.report" :role="mg.report.failed_count ? 'alert' : 'status'" :class="['text-xs', mg.report.failed_count ? 'text-warning' : 'text-success']" data-part="mg-report">
      <template v-if="mg.report.removed">Removed {{ fmt(mg.report.removed) }}, </template>inserted {{ fmt(mg.report.inserted) }}<template v-if="mg.report.upserted || mg.report.updated">, upserted {{ fmt(mg.report.upserted) }}, replaced {{ fmt(mg.report.updated) }}</template> documents. <template v-if="mg.report.failed_count">{{ fmt(mg.report.failed_count) }} refused: {{ mg.report.failed.join('; ') }}</template>
    </p>
  </section>
</template>
