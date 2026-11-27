<script setup lang="ts">
// Bulk import (D17, E6): a folder or zip of CSV, TSV and JSON files. Each file is matched to a table by its name
// (changeable), mapped with D6's preview, or given a reviewed CREATE TABLE when its table isn't there (types from
// the export's manifest, else read from the values). One run loads them all, each file in its own transaction.
import { FileArchive, FolderOpen } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Progress } from '@/components/ui/progress'
import { Textarea } from '@/components/ui/textarea'

const props = defineProps<{ tabKey: string, connId: string, tables: { schema: string, name: string }[] }>()
const bulk = useBulkStore(props.tabKey)
type Row = (typeof bulk.rows)[number]
const exists = (r: Row) => props.tables.some(t => t.schema.toLowerCase() === r.schema.toLowerCase() && t.name.toLowerCase() === r.table.toLowerCase())
const fmt = (n: number | null | undefined) => Math.round(n ?? 0).toLocaleString('en-US')
const included = computed(() => bulk.rows.filter(r => r.include))
function changed(r: Row) {
  // Another table: the mapping or the CREATE TABLE is read again.
  r.preview = null
  r.ddl = null
  r.mapping = []
}
async function toggle(r: Row) {
  r.open = !r.open
  if (r.open) {
    if (exists(r) && !r.preview) await bulk.preview(props.connId, r)
    if (!exists(r) && r.ddl === null) await bulk.infer(props.connId, r)
  }
}
</script>

<template>
  <section class="flex flex-col gap-3 rounded-lg border border-border bg-surface p-4 text-sm" aria-label="Import many" :data-testid="tid('bulk-import')">
    <h2 class="text-sm font-semibold">Import many files</h2>
    <div class="flex gap-2">
      <Button size="sm" variant="secondary" :disabled="bulk.running" @click="bulk.pickImport(connId, false)"><FolderOpen class="size-3.5" aria-hidden="true" />Choose a folder…</Button>
      <Button size="sm" variant="secondary" :disabled="bulk.running" @click="bulk.pickImport(connId, true)"><FileArchive class="size-3.5" aria-hidden="true" />Choose a zip…</Button>
    </div>
    <p v-if="bulk.source" class="text-xs text-muted">{{ bulk.source.name }}<template v-if="bulk.source.from"> · exported by Dalan from {{ bulk.source.from }}</template> · {{ bulk.rows.length }} files<template v-if="bulk.source.skipped.length"> · not read: {{ bulk.source.skipped.join(', ') }}</template></p>
    <ul v-if="bulk.rows.length" class="m-0 flex max-h-[420px] list-none flex-col gap-1 overflow-auto p-0" aria-label="Files to import">
      <li v-for="r in bulk.rows" :key="r.file.id" class="rounded-md border border-border p-2 text-xs" :data-testid="tid(`bulk-file-${r.file.name}`)">
        <div class="flex flex-wrap items-center gap-2">
          <Checkbox v-model="r.include" :aria-label="`Import ${r.file.name}`" />
          <span class="min-w-[12rem] flex-1 truncate font-mono" :title="r.file.name">{{ r.file.name }}</span>
          <span class="text-muted">into</span>
          <Input v-model="r.schema" class="h-7 w-24 font-mono text-xs" :aria-label="`Schema for ${r.file.name}`" @change="changed(r)" />
          <Input v-model="r.table" class="h-7 w-36 font-mono text-xs" :aria-label="`Table for ${r.file.name}`" @change="changed(r)" />
          <span :class="exists(r) ? 'text-muted' : 'text-warning'" data-part="where">{{ exists(r) ? 'existing table' : 'new table' }}</span>
          <Button size="sm" variant="ghost" :aria-expanded="r.open" @click="toggle(r)">{{ exists(r) ? 'Mapping' : 'CREATE TABLE' }}</Button>
        </div>
        <p v-if="r.error" role="alert" class="mt-1 text-danger">{{ r.error }}</p>
        <div v-if="r.open" class="mt-2 flex flex-col gap-1">
          <label class="flex items-center gap-1.5"><Checkbox v-model="r.header" @update:model-value="changed(r); toggle(r); toggle(r)" />First row is a header</label>
          <template v-if="exists(r) && r.preview">
            <p class="text-muted">{{ fmt(r.preview.total_rows) }} rows. Each file column goes into the table column chosen (or is left out).</p>
            <div v-for="(c, i) in r.preview.file_columns" :key="i" class="flex items-center gap-2">
              <span class="w-36 truncate font-mono">{{ c }}</span><span class="text-muted">→</span>
              <select v-model="r.mapping[i]" class="h-7 rounded-md border border-border-input bg-bg px-1" :aria-label="`Column for ${c} of ${r.file.name}`">
                <option :value="null">Leave out</option>
                <option v-for="t in r.preview.table_columns.filter(x => !x.generated)" :key="t.name" :value="t.name">{{ t.name }} · {{ t.type_name }}</option>
              </select>
            </div>
          </template>
          <template v-else-if="!exists(r) && r.ddl !== null">
            <p class="text-muted">The table is made first with this statement (review it; it must stay one CREATE TABLE).</p>
            <Textarea v-model="r.ddl" class="min-h-28 font-mono text-xs" :aria-label="`CREATE TABLE for ${r.file.name}`" />
          </template>
        </div>
      </li>
    </ul>
    <p v-if="bulk.error" role="alert" class="text-xs text-danger">{{ bulk.error }}</p>
    <section v-if="bulk.pending" class="flex flex-col gap-2 rounded-md border border-border bg-bg p-3 text-xs" aria-label="Review the import" :data-testid="tid('bulk-import-plan')">
      <p class="font-medium">Load {{ fmt(bulk.pending.files) }} files ({{ fmt(bulk.pending.rows) }} rows) into {{ bulk.pending.db_name }}<template v-if="bulk.pending.creates">, creating {{ fmt(bulk.pending.creates) }} {{ bulk.pending.creates === 1 ? 'table' : 'tables' }} first</template>?</p>
      <p class="text-muted">Each file loads in one transaction: a file the server refuses changes nothing, and the others still load.</p>
      <p v-if="bulk.pending.environment === 'production'" class="text-warning">{{ bulk.pending.db_name }} is a production database.</p>
      <div class="flex justify-end gap-2"><Button size="sm" variant="secondary" @click="bulk.cancelImport(connId)">Cancel</Button><Button size="sm" :loading="bulk.running" :data-testid="tid('bulk-import-commit')" @click="bulk.commitImport(connId)">Import</Button></div>
    </section>
    <div v-if="bulk.running && bulk.progress" class="flex flex-col gap-1">
      <Progress :model-value="((bulk.progress.index ?? 0) / (bulk.progress.total || 1)) * 100" aria-label="Bulk import progress" />
      <p role="status" class="text-xs text-muted">{{ fmt((bulk.progress.index ?? 0) + 1) }} of {{ fmt(bulk.progress.total) }}: <span class="font-mono">{{ bulk.progress.table }}</span></p>
    </div>
    <table v-if="bulk.report" class="w-full text-left text-xs" aria-label="Import report">
      <thead><tr class="text-muted"><th class="py-1 font-normal">File</th><th class="font-normal">Table</th><th class="text-right font-normal">Rows</th></tr></thead>
      <tbody>
        <tr v-for="x in bulk.report" :key="x.name" class="border-t border-border">
          <td class="py-1 font-mono">{{ x.name }}</td>
          <td class="font-mono">{{ x.table }}<span v-if="x.created" class="text-muted"> (new)</span></td>
          <td :class="['text-right', x.error ? 'text-danger' : 'text-success']">{{ x.error ?? fmt(x.rows) }}</td>
        </tr>
      </tbody>
    </table>
    <div v-if="bulk.rows.length && !bulk.pending" class="flex justify-end"><Button :disabled="!included.length || bulk.running" :data-testid="tid('bulk-import-review')" @click="bulk.prepareImport(connId, exists)">Review the import</Button></div>
  </section>
</template>
