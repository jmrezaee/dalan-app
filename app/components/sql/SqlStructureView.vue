<script setup lang="ts">
// S38 "Table structure": columns, indexes, foreign keys, triggers and the DDL of one table, and changes to them. A
// change here is not run: it joins the pending list as a typed operation. "Review and apply" asks Rust for the
// statements (the gate decides what each needs), shows them, and applies them in one transaction.
import { Copy, Plus, Trash2 } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { TableTab } from '~/stores/sql'
import type { StructureOp_Deserialize as Op } from '~/types/bindings'

const props = defineProps<{ tabKey: string, tab: TableTab }>()
const store = useSqlStore(props.tabKey)
const databases = useDatabasesStore()
const t = computed(() => props.tab)
const s = computed(() => t.value.structure)
const readOnly = computed(() => (databases.rows.find(d => d.id === store.dbId)?.write_mode ?? 'confirm') === 'read_only' || t.value.kind !== 'table')
const section = computed(() => t.value.view)

const bytes = (raw: number | null) => {
  const n = raw ?? 0
  return (n >= 1 << 30 ? `${(n / (1 << 30)).toFixed(1)} GB` : n >= 1 << 20 ? `${(n / (1 << 20)).toFixed(1)} MB` : n >= 1024 ? `${Math.round(n / 1024)} kB` : `${n} B`)
}
const fmt = (n: number | null) => (n ?? 0).toLocaleString('en-US')
const ago = (ms: number | null) => {
  if (ms === null) return 'never vacuumed'
  const h = Math.floor((Date.now() - ms) / 3_600_000)
  return h < 1 ? 'vacuumed just now' : h < 48 ? `last vacuum ${h} h ago` : `last vacuum ${Math.floor(h / 24)} days ago`
}

/** What a column's constraints say, in one line: `pk`, `fk → orders.id`, `check …`, `unique`. */
function constraintsOf(name: string) {
  return (s.value?.constraints ?? []).filter(c => c.columns.includes(name)).map((c) => {
    if (c.kind === 'p') return 'pk'
    if (c.kind === 'u') return 'unique'
    if (c.kind === 'f') return `fk → ${/REFERENCES ([^ ]+)/.exec(c.definition)?.[1] ?? ''}`
    return c.definition.replace(/^CHECK /, 'check ').replace(/^\((.*)\)$/, '$1')
  }).join(' · ')
}
const keys = computed(() => (s.value?.constraints ?? []).filter(c => c.kind === 'f'))
const constraintBacked = (index: string) => (s.value?.constraints ?? []).some(c => c.name === index)

// ---- Pending operations
const dropped = (name: string) => t.value.ops.some(o => o.op === 'drop_column' && o.name === name)
const opText = (o: Op) => {
  switch (o.op) {
    case 'add_column': return `Add column ${o.name} ${o.type_name}${o.nullable ? '' : ' not null'}${o.default ? ` default ${o.default}` : ''}`
    case 'drop_column': return `Drop column ${o.name}`
    case 'rename_column': return `Rename ${o.from} to ${o.to}`
    case 'change_type': return `Change ${o.column} to ${o.type_name}`
    case 'set_nullable': return `${o.column}: ${o.nullable ? 'allow NULL' : 'require a value'}`
    case 'set_default': return o.default ? `${o.column}: default ${o.default}` : `${o.column}: no default`
    case 'create_index': return `Create ${o.unique ? 'unique ' : ''}index ${o.name} on (${o.columns.join(', ')})`
    case 'drop_index': return `Drop index ${o.name}`
    case 'drop_constraint': return `Drop constraint ${o.name}`
    case 'add_foreign_key': return `Add foreign key ${o.name}: (${o.columns.join(', ')}) → ${o.ref_table} (${o.ref_columns.join(', ')})`
  }
}

// ---- Editing one column
const editing = ref<string | null>(null)
const draft = reactive({ name: '', type_name: '', nullable: true, default: '' })
function startEdit(c: NonNullable<typeof s.value>['columns'][number]) {
  editing.value = c.name
  Object.assign(draft, { name: c.name, type_name: c.type_name, nullable: c.nullable, default: c.default ?? '' })
}
function saveEdit() {
  const c = s.value?.columns.find(x => x.name === editing.value)
  if (!c) return
  let col = c.name
  if (draft.name.trim() && draft.name !== c.name) { store.addOp(t.value, { op: 'rename_column', from: c.name, to: draft.name.trim() }); col = draft.name.trim() }
  if (draft.type_name.trim() && draft.type_name !== c.type_name) store.addOp(t.value, { op: 'change_type', column: col, type_name: draft.type_name.trim() })
  if (draft.nullable !== c.nullable) store.addOp(t.value, { op: 'set_nullable', column: col, nullable: draft.nullable })
  if (draft.default !== (c.default ?? '')) store.addOp(t.value, { op: 'set_default', column: col, default: draft.default.trim() || null })
  editing.value = null
}

// ---- New column, index and foreign key
const add = reactive({ name: '', type_name: 'text', nullable: true, default: '' })
function addColumn() {
  if (!add.name.trim() || !add.type_name.trim()) return
  store.addOp(t.value, { op: 'add_column', name: add.name.trim(), type_name: add.type_name.trim(), nullable: add.nullable, default: add.default.trim() || null })
  Object.assign(add, { name: '', type_name: 'text', nullable: true, default: '' })
}
const idx = reactive({ name: '', columns: '', unique: false })
function addIndex() {
  const cols = idx.columns.split(',').map(x => x.trim()).filter(Boolean)
  if (!idx.name.trim() || !cols.length) return
  store.addOp(t.value, { op: 'create_index', name: idx.name.trim(), columns: cols, unique: idx.unique })
  Object.assign(idx, { name: '', columns: '', unique: false })
}
const fk = reactive({ name: '', columns: '', ref: '', refColumns: '', onDelete: '' })
function addFk() {
  const cols = fk.columns.split(',').map(x => x.trim()).filter(Boolean)
  const refCols = fk.refColumns.split(',').map(x => x.trim()).filter(Boolean)
  const [refSchema, ...rest] = fk.ref.includes('.') ? fk.ref.split('.') : [t.value.schema, fk.ref]
  if (!fk.name.trim() || !cols.length || !refCols.length || !rest.length) return
  store.addOp(t.value, { op: 'add_foreign_key', name: fk.name.trim(), columns: cols, ref_schema: refSchema!, ref_table: rest.join('.'), ref_columns: refCols, on_delete: fk.onDelete || null })
  Object.assign(fk, { name: '', columns: '', ref: '', refColumns: '', onDelete: '' })
}

const copied = ref('')
async function copy(text: string, what: string) {
  await navigator.clipboard.writeText(text)
  copied.value = what
  setTimeout(() => { copied.value = '' }, 1600)
}
const copyMigration = async () => {
  try { await copy(await store.migrationText(t.value), 'the migration') }
  catch (e) { t.value.structureError = (e as { message?: string }).message ?? 'Couldn\'t build the migration.' }
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col" :data-testid="tid('sql-structure')">
    <p v-if="t.structureError" role="alert" class="shrink-0 border-b border-border px-3 py-1.5 text-xs text-danger">{{ t.structureError }}</p>
    <div v-if="!s && !t.structureError" class="p-4 text-sm text-muted" aria-busy="true">Reading the structure…</div>
    <div v-else-if="s" class="min-h-0 flex-1 overflow-auto p-3 text-sm">
      <p class="mb-3 text-xs text-muted" data-part="stats">
        <template v-if="s.stats.rows_estimate !== null">{{ fmt(s.stats.rows_estimate) }} rows · </template>{{ bytes(s.stats.table_bytes) }} table · {{ bytes(s.stats.index_bytes) }} indexes<template v-if="!store.isMy"> · {{ ago(s.stats.last_vacuum) }}</template>
      </p>
      <p v-if="readOnly && t.kind === 'table'" class="mb-3 text-xs text-muted">This database is read-only in Dalan, so the structure can be read but not changed.</p>

      <section v-if="section === 'structure'" aria-label="Columns">
        <h3 class="mb-1.5 text-2xs font-medium uppercase tracking-wider text-muted">Columns</h3>
        <table class="w-full border-collapse text-xs">
          <thead><tr class="text-left text-muted"><th class="h-7 font-medium">Name</th><th class="font-medium">Type</th><th class="font-medium">Null</th><th class="font-medium">Default</th><th class="font-medium">Constraints</th><th class="w-24" /></tr></thead>
          <tbody>
            <tr v-for="c in s.columns" :key="c.name" :class="['border-t border-surface-raised', dropped(c.name) && 'line-through opacity-60']" :data-testid="tid(`structure-col-${c.name}`)">
              <template v-if="editing === c.name">
                <td class="py-1 pr-2"><Input v-model="draft.name" class="h-7 font-mono text-xs" :aria-label="`Name of ${c.name}`" /></td>
                <td class="pr-2"><Input v-model="draft.type_name" class="h-7 font-mono text-xs" :aria-label="`Type of ${c.name}`" /></td>
                <td><input v-model="draft.nullable" type="checkbox" :aria-label="`${c.name} allows NULL`"></td>
                <td class="pr-2"><Input v-model="draft.default" class="h-7 font-mono text-xs" :aria-label="`Default of ${c.name}`" /></td>
                <td class="text-muted">{{ constraintsOf(c.name) }}</td>
                <td class="text-right"><Button size="xs" @click="saveEdit">Done</Button></td>
              </template>
              <template v-else>
                <td class="h-8 pr-2 font-mono">{{ c.name }}</td>
                <td class="pr-2 font-mono text-muted">{{ c.type_name }}</td>
                <td>{{ c.nullable ? 'yes' : 'no' }}</td>
                <td class="pr-2 font-mono text-muted">{{ c.generated ?? c.default ?? '' }}</td>
                <td class="text-muted">{{ constraintsOf(c.name) }}</td>
                <td class="text-right">
                  <template v-if="!readOnly">
                    <Button v-if="!dropped(c.name)" variant="ghost" size="xs" :aria-label="`Edit ${c.name}`" @click="startEdit(c)">Edit</Button>
                    <Button variant="ghost" size="xs" :aria-label="`${dropped(c.name) ? 'Keep' : 'Drop'} ${c.name}`" @click="dropped(c.name) ? store.removeOp(t, t.ops.findIndex(o => o.op === 'drop_column' && o.name === c.name)) : store.addOp(t, { op: 'drop_column', name: c.name })"><Trash2 /></Button>
                  </template>
                </td>
              </template>
            </tr>
          </tbody>
        </table>
        <form v-if="!readOnly" class="mt-2 flex flex-wrap items-center gap-2" aria-label="New column" @submit.prevent="addColumn">
          <Input v-model="add.name" class="h-7 w-36 font-mono text-xs" placeholder="new_column" aria-label="New column name" autocomplete="off" />
          <Input v-model="add.type_name" class="h-7 w-36 font-mono text-xs" aria-label="New column type" autocomplete="off" />
          <label class="flex items-center gap-1 text-xs text-muted"><input v-model="add.nullable" type="checkbox">NULL</label>
          <Input v-model="add.default" class="h-7 w-32 font-mono text-xs" placeholder="default" aria-label="New column default" autocomplete="off" />
          <Button type="submit" variant="secondary" size="sm"><Plus />Add column</Button>
        </form>
      </section>

      <section v-else-if="section === 'indexes'" aria-label="Indexes">
        <h3 class="mb-1.5 text-2xs font-medium uppercase tracking-wider text-muted">Indexes</h3>
        <p v-if="!s.indexes.length" class="text-muted">This table has no indexes.</p>
        <table v-else class="w-full border-collapse text-xs">
          <thead><tr class="text-left text-muted"><th class="h-7 font-medium">Name</th><th class="font-medium">Definition</th><th class="font-medium">Size</th><th class="font-medium">Use</th><th class="w-12" /></tr></thead>
          <tbody>
            <tr v-for="i in s.indexes" :key="i.name" class="border-t border-surface-raised">
              <td class="h-8 pr-2 font-mono">{{ i.name }}</td>
              <td class="pr-2 font-mono text-muted">{{ i.definition.replace(/^CREATE (UNIQUE )?INDEX \S+ ON \S+ /, '') }}<span v-if="i.unique" class="ml-1 text-text">unique</span></td>
              <td>{{ bytes(i.bytes) }}</td>
              <td :class="i.scans === 0 ? 'text-warning' : ''">{{ (i.scans ?? 0) < 0 ? '' : i.scans === 0 ? 'no scans yet' : `${fmt(i.scans)} scans` }}</td>
              <td class="text-right"><Button v-if="!readOnly && !constraintBacked(i.name)" variant="ghost" size="xs" :aria-label="`Drop ${i.name}`" @click="store.addOp(t, { op: 'drop_index', name: i.name })"><Trash2 /></Button></td>
            </tr>
          </tbody>
        </table>
        <form v-if="!readOnly" class="mt-2 flex flex-wrap items-center gap-2" aria-label="New index" @submit.prevent="addIndex">
          <Input v-model="idx.name" class="h-7 w-44 font-mono text-xs" placeholder="index_name" aria-label="New index name" autocomplete="off" />
          <Input v-model="idx.columns" class="h-7 w-48 font-mono text-xs" placeholder="column, column" aria-label="New index columns" autocomplete="off" />
          <label class="flex items-center gap-1 text-xs text-muted"><input v-model="idx.unique" type="checkbox">unique</label>
          <Button type="submit" variant="secondary" size="sm"><Plus />New index</Button>
        </form>
      </section>

      <section v-else-if="section === 'keys'" aria-label="Foreign keys">
        <h3 class="mb-1.5 text-2xs font-medium uppercase tracking-wider text-muted">Foreign keys</h3>
        <p v-if="!keys.length" class="text-muted">This table has no foreign keys.</p>
        <ul v-else class="m-0 flex list-none flex-col p-0 text-xs">
          <li v-for="k in keys" :key="k.name" class="flex h-8 items-center gap-3 border-t border-surface-raised">
            <span class="w-52 truncate font-mono">{{ k.name }}</span><span class="min-w-0 flex-1 truncate font-mono text-muted">{{ k.definition }}</span>
            <Button v-if="!readOnly" variant="ghost" size="xs" :aria-label="`Drop ${k.name}`" @click="store.addOp(t, { op: 'drop_constraint', name: k.name })"><Trash2 /></Button>
          </li>
        </ul>
        <form v-if="!readOnly" class="mt-2 flex flex-wrap items-center gap-2" aria-label="New foreign key" @submit.prevent="addFk">
          <Input v-model="fk.name" class="h-7 w-40 font-mono text-xs" placeholder="constraint_name" aria-label="New key name" autocomplete="off" />
          <Input v-model="fk.columns" class="h-7 w-36 font-mono text-xs" placeholder="column" aria-label="New key columns" autocomplete="off" />
          <Input v-model="fk.ref" class="h-7 w-36 font-mono text-xs" placeholder="schema.table" aria-label="New key table" autocomplete="off" />
          <Input v-model="fk.refColumns" class="h-7 w-32 font-mono text-xs" placeholder="id" aria-label="New key referenced columns" autocomplete="off" />
          <select v-model="fk.onDelete" class="h-7 rounded-md border border-border-input bg-bg px-2 text-xs" aria-label="On delete">
            <option value="">on delete: default</option><option>cascade</option><option>restrict</option><option>set null</option>
          </select>
          <Button type="submit" variant="secondary" size="sm"><Plus />Add foreign key</Button>
        </form>
      </section>

      <section v-else-if="section === 'triggers'" aria-label="Triggers">
        <h3 class="mb-1.5 text-2xs font-medium uppercase tracking-wider text-muted">Triggers</h3>
        <p v-if="!s.triggers.length" class="text-muted">This table has no triggers.</p>
        <ul v-else class="m-0 flex list-none flex-col p-0 text-xs">
          <li v-for="g in s.triggers" :key="g.name" class="flex min-h-8 items-center gap-3 border-t border-surface-raised py-1">
            <span class="w-52 truncate font-mono">{{ g.name }}</span><span class="min-w-0 flex-1 break-all font-mono text-muted">{{ g.definition }}</span><span :class="g.enabled ? 'text-success' : 'text-warning'">{{ g.enabled ? 'enabled' : 'disabled' }}</span>
          </li>
        </ul>
      </section>

      <section v-else aria-label="DDL">
        <div class="mb-1.5 flex items-center"><h3 class="flex-1 text-2xs font-medium uppercase tracking-wider text-muted">DDL</h3><Button variant="ghost" size="xs" @click="copy(s.ddl, 'the DDL')"><Copy />Copy</Button></div>
        <pre class="overflow-auto whitespace-pre rounded-md border border-border bg-surface p-3 font-mono text-xs" :data-testid="tid('structure-ddl')">{{ s.ddl }}</pre>
      </section>
    </div>

    <div v-if="t.ops.length" class="shrink-0 border-t border-warning/40 bg-warning/10 px-3 py-2 text-sm" :data-testid="tid('structure-pending')">
      <div class="flex items-center gap-3">
        <b class="font-medium">Pending change</b><span class="text-muted">{{ t.ops.length }} {{ t.ops.length === 1 ? 'operation' : 'operations' }}</span>
        <span v-if="copied" class="text-xs text-success" role="status">Copied {{ copied }}</span>
        <span class="ml-auto flex gap-2">
          <Button variant="ghost" size="sm" @click="t.ops = []">Discard</Button>
          <Button variant="secondary" size="sm" @click="copyMigration">Copy as migration</Button>
          <Button size="sm" @click="store.reviewStructure(t)">Review and apply</Button>
        </span>
      </div>
      <ul class="m-0 mt-1 flex list-none flex-col gap-0.5 p-0 text-xs">
        <li v-for="(o, i) in t.ops" :key="i" class="flex items-center gap-2"><span class="min-w-0 flex-1 truncate font-mono">{{ opText(o) }}</span><Button variant="ghost" size="xs" :aria-label="`Remove: ${opText(o)}`" @click="store.removeOp(t, i)">Remove</Button></li>
      </ul>
    </div>
    <SqlStructureDialog :tab-key="tabKey" :tab="t" />
  </div>
</template>
