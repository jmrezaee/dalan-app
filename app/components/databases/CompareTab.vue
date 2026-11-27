<script setup lang="ts">
// Compare (D16, E3): two schemas of open PostgreSQL tabs (or MySQL/MariaDB databases), or a saved snapshot, as a
// list of differences with both DDLs side by side. "Sync" makes the target match the source through its gate (a drop
// or a type change asks for the typed name); the script can be opened in a query tab or saved as a migration file.
import { Camera, FileDown, GitCompareArrows, SquareTerminal, Trash2 } from '@lucide/vue'
import { Button, buttonVariants } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { DiffItem_Serialize as DiffItem, Engine } from '~/types/bindings'
import type { SideChoice } from '~/stores/compare'

const props = defineProps<{ tabKey: string, engine: Engine, dbName: string }>()
const store = useCompareStore(props.tabKey)
const sessions = useSessionsStore()
const databases = useDatabasesStore()
const family = (e: string | undefined) => (e === 'mariadb' ? 'mysql' : e)
const my = computed(() => family(props.engine) === 'mysql')

// Open tabs of the same engine family, with their schemas (MySQL: databases).
const tabs = computed(() => sessions.sessions
  .filter(s => s.kind === 'sql' && s.dbId)
  .map((s) => {
    const db = databases.rows.find(d => d.id === s.dbId)
    const st = useSqlStore(s.key)
    return { key: s.key, name: db?.name ?? s.label, engine: db?.engine, ready: st.status === 'ready', schemas: st.schemas.map(x => x.name) }
  })
  .filter(t => family(t.engine) === family(props.engine)))
const schemasOf = (key: string) => tabs.value.find(t => t.key === key)?.schemas ?? []
const snapshotsHere = computed(() => store.snapshots.filter(s => (s.family === 'my') === my.value))
const when = (ms: number) => new Date(ms).toLocaleString()

function fill(c: SideChoice, preferOther: boolean) {
  if (!tabs.value.some(t => t.key === c.tabKey)) c.tabKey = (preferOther ? tabs.value.find(t => t.key !== props.tabKey)?.key : undefined) ?? props.tabKey
  const names = schemasOf(c.tabKey)
  if (!names.includes(c.schema)) c.schema = names.find(n => n === (my.value ? null : 'public')) ?? names[0] ?? ''
  if (!snapshotsHere.value.some(s => s.id === c.snapshotId)) c.snapshotId = snapshotsHere.value[0]?.id ?? ''
}
watch([tabs, snapshotsHere], () => { fill(store.source, false); fill(store.target, true) }, { immediate: true, deep: true })
onMounted(() => { void store.loadSnapshots() })
const ready = (c: SideChoice) => (c.kind === 'snapshot' ? !!c.snapshotId : !!c.schema && !!tabs.value.find(t => t.key === c.tabKey)?.ready)

// ---- The differences, grouped by table (objects that aren't a table's under "Other objects").
const r = computed(() => store.result)
const groups = computed(() => {
  const m = new Map<string, DiffItem[]>()
  for (const i of r.value?.items ?? []) {
    const k = i.kind === 'table' ? i.name : i.table ?? ''
    m.set(k, [...(m.get(k) ?? []), i])
  }
  return [...m].sort(([a], [b]) => (a === '' ? 1 : b === '' ? -1 : a.localeCompare(b)))
})
const counts = computed(() => {
  const items = r.value?.items ?? []
  return { added: items.filter(i => i.change === 'added').length, removed: items.filter(i => i.change === 'removed').length, changed: items.filter(i => i.change === 'changed').length }
})
const CHANGE = { added: ['Added', 'text-success'], removed: ['Removed', 'text-danger'], changed: ['Changed', 'text-warning'] } as const
const KIND: Record<string, string> = { table: 'table', column: 'column', primary_key: 'primary key', unique: 'unique key', check: 'check', foreign_key: 'foreign key', exclusion: 'exclusion', index: 'index', enum: 'enum', sequence: 'sequence', view: 'view', function: my.value ? 'routine' : 'function', trigger: 'trigger' }
function toggle(id: string) {
  const next = new Set(store.chosen)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  store.chosen = next
}
const setAll = (on: boolean) => { store.chosen = new Set(on ? (r.value?.items ?? []).map(i => i.id) : []) }
const shown = computed(() => {
  const id = store.selected
  if (!id || !r.value) return null
  if (id.startsWith('ddl:')) {
    const t = r.value.tables.find(x => x.name === id.slice(4))
    return t ? { title: `Table ${t.name}`, source: t.source, target: t.target } : null
  }
  const i = r.value.items.find(x => x.id === id)
  return i ? { title: `${KIND[i.kind] ?? i.kind} ${i.table ? `${i.table}.` : ''}${i.name}`, source: i.source_ddl, target: i.target_ddl } : null
})

async function openInQuery() {
  const text = await store.script()
  if (!text) return
  const st = useSqlStore(store.target.kind === 'database' ? store.target.tabKey : props.tabKey)
  st.newQuery(text, 'Sync script')
  sessions.activeKey = store.target.kind === 'database' ? store.target.tabKey : props.tabKey
}
const typedOk = computed(() => !store.plan || store.plan.level !== 'typed' || store.typed === store.plan.typed_name)
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col overflow-auto p-4 text-sm" :data-testid="tid('compare-tab')">
    <div class="flex max-w-[1100px] flex-col gap-4">
      <div class="grid gap-3 md:grid-cols-2">
        <section v-for="[c, label, hint] in ([[store.source, 'Source', 'How it should be'], [store.target, 'Target', 'What a sync changes']] as const)" :key="label" class="flex flex-col gap-2 rounded-lg border border-border bg-surface p-3" :aria-label="label">
          <h3 class="text-xs font-medium">{{ label }} <span class="font-normal text-muted">· {{ hint }}</span></h3>
          <div role="radiogroup" :aria-label="`${label} is`" class="flex gap-3 text-xs">
            <label class="flex items-center gap-1.5"><input v-model="c.kind" type="radio" :name="`${label}-kind`" value="database" class="accent-primary">A database</label>
            <label class="flex items-center gap-1.5"><input v-model="c.kind" type="radio" :name="`${label}-kind`" value="snapshot" class="accent-primary" :disabled="!snapshotsHere.length">A snapshot</label>
          </div>
          <template v-if="c.kind === 'database'">
            <select v-model="c.tabKey" class="h-8 rounded-md border border-border-input bg-bg px-2" :aria-label="`${label} database`">
              <option v-for="t in tabs" :key="t.key" :value="t.key" :disabled="!t.ready">{{ t.name }}{{ t.ready ? '' : ' (connecting…)' }}</option>
            </select>
            <select v-model="c.schema" class="h-8 rounded-md border border-border-input bg-bg px-2" :aria-label="`${label} ${my ? 'database on the server' : 'schema'}`">
              <option v-for="s in schemasOf(c.tabKey)" :key="s" :value="s">{{ s }}</option>
            </select>
            <div class="flex justify-end"><Button size="sm" variant="ghost" :disabled="!ready(c) || store.busy" @click="store.snapshot(c)"><Camera class="size-3.5" aria-hidden="true" />Save a snapshot</Button></div>
          </template>
          <template v-else>
            <select v-model="c.snapshotId" class="h-8 rounded-md border border-border-input bg-bg px-2" :aria-label="`${label} snapshot`">
              <option v-for="s in snapshotsHere" :key="s.id" :value="s.id">{{ s.database_name }} · {{ s.schema }} · {{ when(s.captured_at ?? 0) }}</option>
            </select>
            <div class="flex justify-end"><Button size="sm" variant="ghost" :disabled="!c.snapshotId" :aria-label="`Delete this ${label.toLowerCase()} snapshot`" @click="store.removeSnapshot(c.snapshotId)"><Trash2 class="size-3.5" aria-hidden="true" />Delete snapshot</Button></div>
          </template>
        </section>
      </div>
      <div class="flex items-center gap-3">
        <Button :disabled="!ready(store.source) || !ready(store.target)" :loading="store.busy && !store.plan" :data-testid="tid('compare-run')" @click="store.compare()"><GitCompareArrows class="size-4" aria-hidden="true" />Compare</Button>
        <p v-if="store.note" role="status" class="text-xs text-success">{{ store.note }}</p>
      </div>
      <p v-if="store.error" role="alert" class="rounded-md border border-danger bg-danger/10 p-2 text-xs text-danger">{{ store.error }}</p>

      <template v-if="r">
        <p role="status" class="text-xs text-muted" data-part="summary">
          <template v-if="!r.items.length">No differences: {{ r.target_label }} matches {{ r.source_label }}.</template>
          <template v-else>{{ r.items.length }} differences · <span class="text-success">{{ counts.added }} added</span> · <span class="text-danger">{{ counts.removed }} removed</span> · <span class="text-warning">{{ counts.changed }} changed</span>. Added and removed say what a sync does to {{ r.target_label }}.</template>
        </p>
        <div v-if="r.items.length" class="grid min-h-0 gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)]">
          <div class="flex flex-col gap-2">
            <div class="flex gap-2 text-xs"><Button size="sm" variant="ghost" @click="setAll(true)">All</Button><Button size="sm" variant="ghost" @click="setAll(false)">None</Button><span class="self-center text-muted">Removals are off until you tick them.</span></div>
            <ul class="m-0 max-h-[460px] list-none overflow-auto rounded-md border border-border p-1" aria-label="Differences">
              <li v-for="[table, items] in groups" :key="table || '~'" class="mb-1">
                <button v-if="table && r.tables.some(t => t.name === table)" type="button" :class="['w-full rounded px-2 py-1 text-left text-xs font-medium text-muted hover:bg-surface-hover', store.selected === `ddl:${table}` && 'bg-surface-hover text-text']" @click="store.selected = `ddl:${table}`">{{ table }}</button>
                <p v-else class="px-2 py-1 text-xs font-medium text-muted">{{ table || 'Other objects' }}</p>
                <div v-for="i in items" :key="i.id" :class="['flex items-start gap-2 rounded px-2 py-1 text-xs hover:bg-surface-hover', store.selected === i.id && 'bg-surface-hover']" :data-testid="tid(`diff-${i.id}`)">
                  <Checkbox :model-value="store.chosen.has(i.id)" :aria-label="`Include ${KIND[i.kind] ?? i.kind} ${i.name}`" class="mt-0.5" @update:model-value="toggle(i.id)" />
                  <button type="button" class="min-w-0 flex-1 text-left" @click="store.selected = i.id">
                    <span :class="CHANGE[i.change][1]">{{ CHANGE[i.change][0] }}</span> {{ KIND[i.kind] ?? i.kind }} <span class="font-mono">{{ i.name }}</span>
                    <span v-if="i.dangerous" class="ml-1 text-danger">· {{ i.change === 'removed' ? 'drops it' : 'changes data' }}</span>
                    <span v-for="d in i.detail" :key="d" class="block truncate text-muted" :title="d">{{ d }}</span>
                  </button>
                </div>
              </li>
            </ul>
          </div>
          <div v-if="shown" class="flex min-w-0 flex-col gap-1">
            <h4 class="text-xs font-medium">{{ shown.title }}</h4>
            <div class="grid gap-2 md:grid-cols-2">
              <div class="min-w-0"><p class="text-xs text-muted">Source</p><pre class="max-h-[420px] overflow-auto whitespace-pre-wrap rounded-md border border-border bg-bg p-2 font-mono text-xs" aria-label="Source DDL">{{ shown.source ?? '(not there)' }}</pre></div>
              <div class="min-w-0"><p class="text-xs text-muted">Target</p><pre class="max-h-[420px] overflow-auto whitespace-pre-wrap rounded-md border border-border bg-bg p-2 font-mono text-xs" aria-label="Target DDL">{{ shown.target ?? '(not there)' }}</pre></div>
            </div>
          </div>
        </div>
        <div v-if="r.items.length && !store.plan" class="flex flex-wrap justify-end gap-2">
          <Button variant="ghost" :disabled="!store.chosen.size || store.busy" @click="store.saveScript()"><FileDown class="size-4" aria-hidden="true" />Copy as migration…</Button>
          <Button variant="secondary" :disabled="!store.chosen.size || store.busy" @click="openInQuery"><SquareTerminal class="size-4" aria-hidden="true" />Open in a query tab</Button>
          <Button :disabled="!store.chosen.size || !r.can_sync || store.busy" :title="r.can_sync ? undefined : 'A snapshot can\'t be changed'" :data-testid="tid('compare-sync')" @click="store.prepare()">Sync…</Button>
        </div>

        <section v-if="store.plan" class="flex flex-col gap-2 rounded-lg border border-border bg-surface p-3" aria-label="Sync" :data-testid="tid('compare-plan')">
          <h3 class="text-sm font-medium">Make {{ r.target_label }} match {{ r.source_label }}?</h3>
          <p class="text-xs text-muted">{{ store.plan.statements }} statements<template v-if="store.plan.dangerous"> · <span class="text-danger">{{ store.plan.dangerous }} drop or change data</span></template>. {{ my ? 'MySQL and MariaDB can\'t undo DDL: if a statement fails, the ones before it stay.' : 'It runs in one transaction: if a statement fails, nothing changes.' }}</p>
          <p v-if="store.plan.environment === 'production'" class="text-xs text-warning">{{ store.plan.typed_name }} is a production database.</p>
          <pre class="max-h-[320px] overflow-auto whitespace-pre-wrap rounded-md border border-border bg-bg p-2 font-mono text-xs" aria-label="Sync script">{{ store.plan.script }}</pre>
          <label v-if="store.plan.level === 'typed'" class="flex flex-col gap-1 text-xs">Type <span class="font-mono">{{ store.plan.typed_name }}</span> to run it<Input v-model="store.typed" class="h-8 w-72 font-mono" :aria-label="`Type ${store.plan.typed_name} to confirm`" /></label>
          <div class="flex justify-end gap-2">
            <Button variant="secondary" @click="store.cancelPlan()">Cancel</Button>
            <Button :class="store.plan.level === 'typed' ? buttonVariants({ variant: 'destructive' }) : undefined" :disabled="!typedOk" :loading="store.busy" :data-testid="tid('compare-commit')" @click="store.commit()">Run sync</Button>
          </div>
        </section>
        <p v-if="store.report" :role="store.report.failed ? 'alert' : 'status'" :class="['whitespace-pre-wrap rounded-md border p-2 text-xs', store.report.failed ? 'border-danger text-danger' : 'border-border text-success']" data-part="report">
          <template v-if="!store.report.failed">Ran {{ store.report.ran }} statements. Compare again to check.</template>
          <template v-else>{{ store.report.rolled_back ? 'It stopped and nothing changed' : `It stopped after ${store.report.ran} of ${store.report.total} statements` }}: {{ store.report.failed }}</template>
        </p>
      </template>
    </div>
  </div>
</template>
