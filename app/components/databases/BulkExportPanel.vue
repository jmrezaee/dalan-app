<script setup lang="ts">
// Bulk export (D17, E6): many tables or collections at once into a new folder or a .zip, one file each, with a
// manifest.json (and the tables' DDL for SQL). Masking rules apply to every table that has the column. Rust picks
// the place and writes every file; an export that doesn't finish leaves nothing behind.
import { Plus, X } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Progress } from '@/components/ui/progress'
import type { ExportFormat } from '~/types/bindings'

const props = defineProps<{ tabKey: string, connId: string, objects: { schema: string, name: string }[], mongo: boolean }>()
const bulk = useBulkStore(props.tabKey)
const key = (o: { schema: string, name: string }) => `${o.schema}.${o.name}`
const all = computed(() => props.objects.length > 0 && props.objects.every(o => bulk.chosen.has(key(o))))
function toggle(k: string) {
  const next = new Set(bulk.chosen)
  if (next.has(k)) next.delete(k)
  else next.add(k)
  bulk.chosen = next
}
const setAll = () => { bulk.chosen = new Set(all.value ? [] : props.objects.map(key)) }
const picked = computed(() => props.objects.filter(o => bulk.chosen.has(key(o))))
const FORMATS: { id: ExportFormat, label: string }[] = [{ id: 'csv', label: 'CSV' }, { id: 'tsv', label: 'TSV' }, { id: 'json', label: 'JSON lines' }, { id: 'sql', label: 'SQL inserts' }]
const fmt = (n: number | null | undefined) => Math.round(n ?? 0).toLocaleString('en-US')
const noun = computed(() => (props.mongo ? 'collections' : 'tables'))
</script>

<template>
  <section class="flex flex-col gap-3 rounded-lg border border-border bg-surface p-4 text-sm" aria-label="Export many" :data-testid="tid('bulk-export')">
    <h2 class="text-sm font-semibold">Export many {{ noun }}</h2>
    <div class="flex items-center gap-2 text-xs"><Button size="sm" variant="ghost" @click="setAll">{{ all ? 'None' : 'All' }}</Button><span class="text-muted">{{ picked.length }} of {{ objects.length }} chosen</span></div>
    <ul class="m-0 max-h-40 list-none overflow-auto rounded-md border border-border p-1" :aria-label="`${noun} to export`">
      <li v-for="o in objects" :key="key(o)"><label class="flex items-center gap-2 rounded px-2 py-0.5 font-mono text-xs hover:bg-surface-hover"><Checkbox :model-value="bulk.chosen.has(key(o))" @update:model-value="toggle(key(o))" />{{ key(o) }}</label></li>
      <li v-if="!objects.length" class="p-2 text-xs text-muted">Open the {{ mongo ? 'databases' : 'schemas' }} in the tree to list their {{ noun }}.</li>
    </ul>
    <div v-if="!mongo" class="flex flex-wrap items-center gap-3 text-xs">
      <select v-model="bulk.format" class="h-8 rounded-md border border-border-input bg-bg px-2" aria-label="Format of every file">
        <option v-for="f in FORMATS" :key="f.id" :value="f.id">{{ f.label }}</option>
      </select>
      <label class="flex items-center gap-1.5"><Checkbox v-model="bulk.header" :disabled="bulk.format !== 'csv' && bulk.format !== 'tsv'" />Header row</label>
      <label class="flex items-center gap-1.5"><Checkbox v-model="bulk.ddl" />The tables' DDL (schema.sql)</label>
    </div>
    <p v-else class="text-xs text-muted">Collections are written as JSON lines (extended JSON).</p>
    <details v-if="!mongo" class="text-xs">
      <summary class="cursor-default">Mask columns<span v-if="bulk.masks.length" class="text-muted"> · {{ bulk.masks.length }}</span></summary>
      <p class="mt-1 text-muted">A rule applies to every chosen table that has the column.</p>
      <div v-for="(m, i) in bulk.masks" :key="i" class="mt-1 flex items-center gap-2">
        <Input v-model="m.column" class="h-7 w-36 font-mono text-xs" :aria-label="`Masked column ${i + 1} of the bulk export`" placeholder="column" />
        <MaskKindSelect v-model="m.kind" :label="`Masking for bulk column ${i + 1}`" />
        <Input v-if="['fixed', 'truncate', 'date_shift'].includes(m.kind)" v-model="m.value" class="h-7 w-20 text-xs" :aria-label="`Value for bulk column ${i + 1}`" />
        <Button size="sm" variant="ghost" :aria-label="`Remove rule ${i + 1}`" @click="bulk.masks.splice(i, 1)"><X class="size-3.5" aria-hidden="true" /></Button>
      </div>
      <Button size="sm" variant="ghost" class="mt-1" @click="bulk.masks.push({ column: '', kind: 'fake_email', value: '' })"><Plus class="size-3.5" aria-hidden="true" />Add a rule</Button>
    </details>
    <div role="radiogroup" aria-label="Export into" class="flex gap-4 text-xs">
      <label class="flex items-center gap-1.5"><input v-model="bulk.zip" type="radio" name="bulk-into" :value="false" class="accent-primary">A new folder</label>
      <label class="flex items-center gap-1.5"><input v-model="bulk.zip" type="radio" name="bulk-into" :value="true" class="accent-primary">A .zip file</label>
    </div>
    <div v-if="bulk.running && bulk.progress" class="flex flex-col gap-1">
      <Progress :model-value="((bulk.progress.index ?? 0) / (bulk.progress.total || 1)) * 100" aria-label="Bulk export progress" />
      <p role="status" class="text-xs text-muted">{{ fmt((bulk.progress.index ?? 0) + 1) }} of {{ fmt(bulk.progress.total) }}: <span class="font-mono">{{ bulk.progress.table }}</span> · {{ fmt(bulk.progress.rows) }} rows</p>
    </div>
    <p v-if="bulk.done" role="status" class="text-xs text-success" data-part="bulk-done">Saved {{ bulk.done.name }}: {{ fmt(bulk.done.files) }} files, {{ fmt(bulk.done.rows) }} rows, and manifest.json.</p>
    <p v-for="w in bulk.done?.warnings ?? []" :key="w" class="text-xs text-warning">{{ w }}</p>
    <p v-if="bulk.error" role="alert" class="text-xs text-danger">{{ bulk.error }}</p>
    <div class="flex justify-end gap-2">
      <Button v-if="bulk.running" variant="secondary" @click="bulk.stop()">Stop</Button>
      <Button :disabled="!picked.length || bulk.running" :data-testid="tid('bulk-export-run')" @click="bulk.exportMany(connId, picked)">Export {{ picked.length }} {{ noun }}…</Button>
    </div>
  </section>
</template>
