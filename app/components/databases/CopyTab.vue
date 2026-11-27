<script setup lang="ts">
// S49 "Copy data" (F32): this database is the source; the target is another open database tab of the same engine
// (or a new database on its server). Rust reads the source in one snapshot on a connection of its own, writes through
// the target's route, and holds the copy for the target's gate (typed when something there is dropped, emptied or
// replaced, or the target is production). The data never touches the window.
import { Copy, Search } from '@lucide/vue'
import { Button, buttonVariants } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Progress } from '@/components/ui/progress'
import type { Engine } from '~/types/bindings'
import { cn } from '@/lib/utils'
import { pickKey } from '~/stores/copy'

const props = defineProps<{ tabKey: string, engine: Engine, connId: string, dbName: string, environment: string }>()
const store = useCopyStore(props.tabKey)
const sessions = useSessionsStore()
const databases = useDatabasesStore()
const mongo = computed(() => props.engine === 'mongodb')
const family = (e: string | undefined) => (e === 'mariadb' ? 'mysql' : e)
// PostgreSQL, MySQL and MariaDB copy to each other; MongoDB to MongoDB.
const fits = (e: string | undefined) => family(e) === family(props.engine) || (family(e) !== 'mongodb' && family(props.engine) !== 'mongodb' && !!e && e !== 'redis')
const fmt = (n: number | null | undefined) => Math.round(n ?? 0).toLocaleString('en-US')

// ---- The source's objects (all schemas or databases, loaded once)
const sql = mongo.value ? null : useSqlStore(props.tabKey)
const mg = mongo.value ? useMongoStore(props.tabKey) : null
onMounted(async () => {
  if (sql) await Promise.all(sql.schemas.map(s => sql.ensureObjects(s)))
  if (mg) await Promise.all(mg.dbs.filter(d => !['admin', 'local', 'config'].includes(d.name)).map(d => mg.ensureColls(d)))
})
watchEffect(() => {
  if (sql) store.offer(sql.schemas.flatMap(s => (s.objects ?? []).filter(o => ['table', 'view', 'matview'].includes(o.kind)).map(o => ({ schema: s.name, name: o.name, kind: o.kind }))))
  if (mg) store.offer(mg.dbs.flatMap(d => (d.colls ?? []).filter(c => c.kind === 'collection' || c.kind === 'view').map(c => ({ schema: d.name, name: c.name, kind: c.kind === 'view' ? 'view' : 'collection' }))))
})
const search = ref('')
const rows = computed(() => Object.values(store.picks).filter(p => !search.value || `${p.schema}.${p.name}`.toLowerCase().includes(search.value.toLowerCase())))
const allOn = computed(() => rows.value.length > 0 && rows.value.every(p => p.checked))
function toggleAll() {
  const on = !allOn.value
  for (const p of rows.value) p.checked = on
}

// ---- Targets: open tabs of the same engine (this one too: into a new database, or under other names)
const targets = computed(() => sessions.sessions
  .filter(s => (s.kind === 'sql' || s.kind === 'mongo') && s.dbId)
  .map((s) => {
    const db = databases.rows.find(d => d.id === s.dbId)
    const st = s.kind === 'mongo' ? useMongoStore(s.key) : useSqlStore(s.key)
    return { key: s.key, name: db?.name ?? s.label, engine: db?.engine, environment: db?.environment ?? 'none', connId: st.connId, ready: st.status === 'ready' }
  })
  .filter(t => fits(t.engine)))
watch(targets, (t) => { if (!t.some(x => x.key === store.targetKey)) store.targetKey = t.find(x => x.key !== props.tabKey)?.key ?? t[0]?.key ?? '' }, { immediate: true })
const target = computed(() => targets.value.find(t => t.key === store.targetKey) ?? null)
const cross = computed(() => !!target.value && family(target.value.engine) !== family(props.engine))
const LABEL: Record<string, string> = { postgres: 'PostgreSQL', mysql: 'MySQL', mariadb: 'MariaDB', mongodb: 'MongoDB' }
watch(cross, (c) => { if (c) store.routines = false })
const pickOf = (source: string) => Object.values(store.picks).find(p => `${p.schema}.${p.name}` === source || p.name === source)
function setType(source: string, col: string, value: string) {
  const p = pickOf(source)
  if (p) p.types[col] = value
}
function setDdl(source: string, value: string) {
  const p = pickOf(source)
  if (p) p.ddl = value
}
const canReview = computed(() => !!target.value?.ready && store.chosen.length > 0 && (!store.targetNew || store.newName.trim().length > 0))

// ---- Review, progress, report
const p = computed(() => store.plan)
const typedOk = computed(() => !p.value || p.value.level !== 'typed' || store.typed === p.value.typed_name)
const pr = computed(() => store.progress)
const pct = computed(() => {
  const x = pr.value
  if (!x || !x.tables) return 0
  const inTable = x.estimate ? Math.min(1, (x.rows ?? 0) / x.estimate) : 0
  return Math.round((((x.index ?? 0) + inTable) / x.tables) * 100)
})
const r = computed(() => store.report)
const check = (t: { source_rows: number | null, target_rows: number | null, status: string }) => (t.status !== 'done' || t.source_rows === null || t.target_rows === null ? '' : t.source_rows === t.target_rows ? 'match' : 'differ')
const MODES: [string, string][] = [['create', 'Create (stop if it exists)'], ['recreate', 'Drop and recreate'], ['empty', 'Empty then load'], ['append', 'Add rows'], ['upsert', 'Insert or replace by key']]
const section = 'flex flex-col gap-2 rounded-lg border border-border bg-surface p-4'
</script>

<template>
  <div class="min-h-0 flex-1 overflow-auto p-4 text-sm" :data-testid="tid('copy-tab')">
    <!-- Choose -->
    <div v-if="store.step === 'choose'" class="flex max-w-[1100px] flex-col gap-4">
      <div class="grid gap-4 md:grid-cols-2">
        <section :class="section" aria-label="Source">
          <h2 class="text-xs font-medium text-muted">From</h2>
          <p class="font-medium">{{ dbName }} <span :class="environment === 'production' ? 'text-warning' : 'text-muted'" class="text-xs">{{ environment === 'none' ? '' : environment }}</span></p>
          <p class="text-xs text-muted">Read in one snapshot{{ mongo ? ' (MongoDB: as it is while the copy runs)' : '' }}, on a connection of its own. Nothing is written here.</p>
        </section>
        <section :class="section" aria-label="Target">
          <h2 class="text-xs font-medium text-muted">To</h2>
          <select v-model="store.targetKey" class="h-8 rounded-md border border-border-input bg-bg px-2 text-sm" aria-label="Target database">
            <option v-for="t in targets" :key="t.key" :value="t.key" :disabled="!t.ready">{{ t.name }} · {{ LABEL[t.engine ?? ''] ?? t.engine }}{{ t.key === tabKey ? ' (this one)' : '' }}{{ t.ready ? '' : ' (connecting…)' }}</option>
          </select>
          <label class="flex items-center gap-2 text-xs"><input v-model="store.targetNew" type="checkbox">Into a new database on that server</label>
          <Input v-if="store.targetNew" v-model="store.newName" class="h-8 w-64 font-mono text-xs" placeholder="name" aria-label="New database name" autocomplete="off" spellcheck="false" />
          <p class="text-xs text-muted">Open the target database in a tab to see it here; copies go between databases {{ mongo ? 'of MongoDB' : 'of PostgreSQL, MySQL and MariaDB, in any direction (between engines the review shows each column\'s type)' }}.</p>
        </section>
      </div>

      <section :class="section" aria-label="What to copy">
        <div class="flex items-center gap-2">
          <h2 class="mr-auto text-xs font-medium text-muted">What to copy · {{ fmt(store.chosen.length) }} chosen</h2>
          <div class="relative"><Search class="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-muted" aria-hidden="true" /><Input v-model="search" class="h-7 w-52 pl-7 text-xs" placeholder="Find a table" aria-label="Find a table" /></div>
          <Button v-if="!mongo" size="sm" variant="ghost" :disabled="!store.chosen.length" :data-testid="tid('copy-related')" @click="store.selectRelated(connId)">Select related</Button>
          <Button size="sm" variant="ghost" @click="toggleAll">{{ allOn ? 'None' : 'All' }}</Button>
        </div>
        <table class="w-full table-fixed border-collapse text-xs" aria-label="Objects to copy">
          <colgroup><col class="w-8"><col><col class="w-44"><col class="w-56"><col class="w-24"></colgroup>
          <thead><tr class="text-left text-muted"><th class="sr-only">Copy</th><th class="font-medium">{{ mongo ? 'Collection' : 'Table' }}</th><th class="font-medium">Name at the target</th><th class="font-medium">Only {{ mongo ? 'documents matching' : 'rows where' }}</th><th class="font-medium">At most</th></tr></thead>
          <tbody>
            <tr v-for="x in rows" :key="pickKey(x)" class="border-t border-surface-raised">
              <td class="h-8"><input v-model="x.checked" type="checkbox" :aria-label="`Copy ${x.schema}.${x.name}`"></td>
              <td class="truncate font-mono" :title="`${x.schema}.${x.name}`"><span class="text-muted">{{ x.schema }}.</span>{{ x.name }}<span v-if="x.kind !== 'table' && x.kind !== 'collection'" class="ml-1 text-muted">{{ x.kind }}</span></td>
              <td><Input v-model="x.rename" :disabled="!x.checked" class="h-7 font-mono text-xs" :placeholder="x.name" :aria-label="`Name at the target for ${x.name}`" /></td>
              <td><Input v-model="x.filter" :disabled="!x.checked || x.kind === 'view' || x.kind === 'matview'" class="h-7 font-mono text-xs" :placeholder="mongo ? 'JSON filter, optional' : 'optional'" :aria-label="`Filter for ${x.name}`" /></td>
              <td><Input v-model="x.limit" :disabled="!x.checked || x.kind === 'view' || x.kind === 'matview'" class="h-7 text-xs" inputmode="numeric" placeholder="all" :aria-label="`Row limit for ${x.name}`" /></td>
            </tr>
          </tbody>
        </table>
        <p v-if="!rows.length" class="text-xs text-muted">{{ search ? 'Nothing matches.' : 'Listing the tables…' }}</p>
      </section>

      <section :class="section" aria-label="Options">
        <fieldset class="flex flex-wrap gap-x-4 gap-y-2">
          <legend class="mb-1 text-xs font-medium text-muted">Tables that already exist at the target</legend>
          <label v-for="m in MODES" :key="m[0]" class="flex items-center gap-2"><input v-model="store.mode" type="radio" :value="m[0]">{{ m[1] }}</label>
        </fieldset>
        <fieldset class="flex flex-wrap gap-x-4 gap-y-2">
          <legend class="mb-1 text-xs font-medium text-muted">What to copy</legend>
          <label class="flex items-center gap-2"><input v-model="store.content" type="radio" value="all">Schema and data</label>
          <label class="flex items-center gap-2"><input v-model="store.content" type="radio" value="schema">Schema only</label>
          <label class="flex items-center gap-2"><input v-model="store.content" type="radio" value="data">Data only</label>
        </fieldset>
        <label class="flex items-center gap-2"><input v-model="store.stopOnError" type="checkbox">Stop at the first table that fails (otherwise skip it and go on)</label>
        <label v-if="!mongo" class="flex items-center gap-2" :class="cross ? 'text-muted' : ''"><input v-model="store.routines" type="checkbox" :disabled="cross">Also copy functions, procedures and triggers{{ cross ? ' (same engine only; between engines they are listed with their DDL)' : '' }}</label>
      </section>
      <p v-if="store.error" role="alert" class="text-danger" data-part="error">{{ store.error }}</p>
      <div class="flex justify-end"><Button :disabled="!canReview" :loading="store.busy" :data-testid="tid('copy-review')" @click="target && store.review(connId, target.connId)"><Copy class="size-4" aria-hidden="true" />Review the copy</Button></div>
    </div>

    <!-- Review -->
    <div v-else-if="store.step === 'review' && p" class="flex max-w-[1000px] flex-col gap-3" :data-testid="tid('copy-plan')">
      <h2 class="text-base font-semibold">Copy {{ fmt(p.tables.length) }} {{ p.tables.length === 1 ? 'object' : 'objects' }} from {{ p.source_name }} to {{ p.target_db }}{{ p.new_database ? ' (new)' : '' }} on {{ p.target_name }}?</h2>
      <p class="text-xs" :class="p.environment === 'production' ? 'text-warning' : 'text-muted'">{{ p.environment === 'production' ? 'The target is a production database.' : p.snapshot ? 'The source is read in one snapshot. Each table is loaded in one transaction: a failure leaves it as it was.' : 'MongoDB has no transaction across a collection: a failure leaves the documents copied so far.' }}</p>
      <p v-if="p.blocked" role="alert" class="rounded-md border border-warning/40 p-3 text-warning" data-part="blocked">{{ p.blocked }}</p>
      <table class="w-full border-collapse text-xs" aria-label="What the copy does">
        <thead><tr class="text-left text-muted"><th class="h-7 font-medium">From</th><th class="font-medium">To</th><th class="font-medium">Does</th><th class="text-right font-medium">Rows (estimate)</th></tr></thead>
        <tbody>
          <tr v-for="t in p.tables" :key="t.source" class="border-t border-surface-raised">
            <td class="h-8 font-mono">{{ t.source }}</td>
            <td class="font-mono">{{ t.target }}</td>
            <td :class="['drop and create', 'empty and load', 'insert or replace'].includes(t.action) && t.exists ? 'text-danger' : ''">{{ t.action }}{{ t.exists ? '' : (t.action === 'create' ? '' : ' (new)') }}</td>
            <td class="text-right tabular-nums text-muted">{{ t.rows_estimate === null ? '' : fmt(t.rows_estimate) }}</td>
          </tr>
        </tbody>
      </table>
      <ul v-if="p.warnings.length" class="m-0 flex list-none flex-col gap-1 p-0 text-xs text-warning" data-part="warnings"><li v-for="w in p.warnings" :key="w">{{ w }}</li></ul>
      <!-- Between engines: each column's mapping, overridable, and the DDL, editable. -->
      <details v-for="t in p.tables.filter(x => x.columns.length)" :key="`map-${t.source}`" class="rounded-md border border-border bg-surface p-3" :data-testid="tid('copy-mapping')">
        <summary class="cursor-default text-xs font-medium">{{ t.source }} → {{ t.target }} · {{ t.columns.length }} columns<span v-if="t.columns.some(c => c.lossy)" class="text-warning"> · {{ t.columns.filter(c => c.lossy).length }} lossy</span></summary>
        <table class="mt-2 w-full table-fixed border-collapse text-xs" :aria-label="`Column types for ${t.source}`">
          <thead><tr class="text-left text-muted"><th class="h-7 w-40 font-medium">Column</th><th class="w-48 font-medium">Source type</th><th class="w-48 font-medium">Target type</th><th class="font-medium">What changes</th></tr></thead>
          <tbody>
            <tr v-for="c in t.columns" :key="c.name" class="border-t border-surface-raised">
              <td class="h-8 truncate font-mono">{{ c.name }}</td>
              <td class="truncate font-mono text-muted" :title="c.source_type">{{ c.source_type }}</td>
              <td><Input :model-value="c.target_type" class="h-7 font-mono text-xs" :aria-label="`Target type for ${c.name}`" @update:model-value="(v: string | number) => setType(t.source, c.name, String(v))" /></td>
              <td :class="c.lossy ? 'text-warning' : 'text-muted'">{{ c.lossy ? `Lossy: ${c.lossy}.` : '' }}</td>
            </tr>
          </tbody>
        </table>
        <label class="mt-2 flex flex-col gap-1 text-xs text-muted">The table it makes (edit it if you need to; it must stay one CREATE TABLE)
          <textarea :value="t.ddl ?? ''" rows="6" spellcheck="false" class="rounded-md border border-border-input bg-bg p-2 font-mono text-xs text-text" :aria-label="`DDL for ${t.target}`" @input="(e: Event) => setDdl(t.source, (e.target as HTMLTextAreaElement).value)" />
        </label>
        <div class="mt-2 flex justify-end"><Button size="sm" variant="secondary" :loading="store.busy" @click="store.review(connId, store.lastTarget)">Apply changes</Button></div>
      </details>
      <label v-if="p.level === 'typed' && !p.blocked" class="flex max-w-md flex-col gap-1 text-xs text-muted">Type {{ p.typed_name }} to confirm
        <Input v-model="store.typed" class="font-mono text-xs text-text" autocomplete="off" spellcheck="false" :data-testid="tid('copy-typed')" />
      </label>
      <p v-if="store.error" role="alert" class="text-danger" data-part="error">{{ store.error }}</p>
      <div class="flex justify-end gap-2">
        <Button variant="secondary" @click="store.reset()">Back</Button>
        <Button :class="p.level === 'typed' ? buttonVariants({ variant: 'destructive' }) : ''" :disabled="!p.pending_id || !typedOk" :data-testid="tid('copy-run')" @click="store.commit(connId)">Copy</Button>
      </div>
    </div>

    <!-- Progress -->
    <div v-else-if="store.step === 'progress'" class="flex max-w-[800px] flex-col gap-3" :data-testid="tid('copy-progress')">
      <p role="status">{{ pr?.phase === 'keys' ? 'Adding keys, indexes and sequences…' : pr ? `Copying ${pr.table} (${fmt((pr.index ?? 0) + 1)} of ${fmt(pr.tables)})…` : 'Starting…' }}</p>
      <Progress :model-value="pct" aria-label="Progress" />
      <p v-if="pr && pr.phase === 'rows'" class="text-xs tabular-nums text-muted">{{ fmt(pr.rows) }} rows{{ pr.estimate ? ` of about ${fmt(pr.estimate)}` : '' }}</p>
      <div class="flex justify-end"><Button variant="secondary" :data-testid="tid('copy-stop')" @click="store.stop()">Stop</Button></div>
    </div>

    <!-- Report -->
    <div v-else-if="store.step === 'report' && r" class="flex max-w-[1000px] flex-col gap-3" :data-testid="tid('copy-report')">
      <p role="status" data-part="outcome" :class="cn(r.outcome === 'done' ? 'text-success' : r.outcome === 'stopped' ? '' : 'text-danger')">
        {{ r.outcome === 'done' ? `Copied into ${r.target_db} in ${Math.max(1, Math.round(r.seconds ?? 0))} s.` : r.outcome === 'stopped' ? 'Stopped. The table being copied was rolled back; the ones marked done are in.' : r.outcome === 'partial' ? 'Some tables failed; the others are in.' : 'A table failed, so the copy stopped. The tables marked done are in.' }}
      </p>
      <table class="w-full border-collapse text-xs" aria-label="Copy report">
        <thead><tr class="text-left text-muted"><th class="h-7 font-medium">From</th><th class="font-medium">To</th><th class="font-medium">Result</th><th class="pr-4 text-right font-medium">Rows</th><th class="font-medium">Row check</th></tr></thead>
        <tbody>
          <tr v-for="t in r.tables" :key="t.source" class="border-t border-surface-raised align-top">
            <td class="h-8 py-1.5 font-mono">{{ t.source }}</td>
            <td class="py-1.5 font-mono">{{ t.target }}</td>
            <td class="py-1.5" :class="t.status === 'failed' ? 'text-danger' : t.status === 'done' ? 'text-success' : 'text-muted'">{{ t.status }}<span v-if="t.error" class="block break-words text-danger">{{ t.error }}</span></td>
            <td class="py-1.5 pr-4 text-right tabular-nums">{{ fmt(t.rows) }}</td>
            <td class="py-1.5" :class="check(t) === 'differ' ? 'text-warning' : 'text-muted'">{{ check(t) === 'match' ? `${fmt(t.target_rows)} on both sides` : check(t) === 'differ' ? `source ${fmt(t.source_rows)}, target ${fmt(t.target_rows)}` : '' }}</td>
          </tr>
        </tbody>
      </table>
      <ul v-if="r.warnings.length" class="m-0 flex list-none flex-col gap-1 p-0 text-xs text-warning" data-part="warnings"><li v-for="w in r.warnings" :key="w">{{ w }}</li></ul>
      <div v-if="r.not_copied.length" data-part="not-copied">
        <h3 class="mb-1 text-xs font-medium text-muted">Not copied (copy these by hand if you need them)</h3>
        <details v-for="x in r.not_copied" :key="x.name" class="text-xs"><summary class="cursor-default font-mono">{{ x.kind }} {{ x.name }}</summary><pre class="mt-1 whitespace-pre-wrap break-all rounded-md border border-border bg-bg p-2 font-mono">{{ x.ddl || 'Its definition couldn\'t be read.' }}</pre></details>
      </div>
      <div class="flex justify-end gap-2">
        <Button v-if="r.outcome !== 'done'" variant="secondary" :data-testid="tid('copy-again')" @click="store.again()">Run again from the failed table</Button>
        <Button @click="store.reset()">Done</Button>
      </div>
    </div>
  </div>
</template>
