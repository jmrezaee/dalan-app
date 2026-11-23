<script setup lang="ts">
// A query tab (S37, F23): the editor on top, the results below. Run the statement at the caret (⌘↵), the selection or
// the whole script (⇧⌘↵); rows stream in while it runs, Stop cancels it on the server. D3 runs reads only: anything
// else is refused with a note until the confirmation dialogs arrive (D4).
import { Check, Copy, History, Play, Save, Square, Star, Trash2, TriangleAlert } from '@lucide/vue'
import { cn } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import type { QueryTab, QueryView } from '~/stores/sql'
import { COPY_FORMATS, copyAs, type CopyFormat } from '~/utils/resultCopy'

const props = defineProps<{ tabKey: string, query: QueryTab }>()
const store = useSqlStore(props.tabKey)
const databases = useDatabasesStore()
const q = computed(() => props.query)
const editor = ref<{ focus: () => void, selected: () => string } | null>(null)

const result = computed(() => (q.value.view.startsWith('r') ? q.value.results[Number(q.value.view.slice(1))] ?? null : null))
const saved = computed(() => databases.saved.filter(s => s.engine === store.engine && s.kind === 'sql' && (!s.database_id || s.database_id === store.dbId)))
const fmt = (n: number) => n.toLocaleString('en-US')

// ---- Running
const run = (mode: 'current' | 'selection' | 'all', selected = '') => store.run(q.value, mode, selected)
function runSelection() {
  const s = editor.value?.selected() ?? ''
  void run('selection', s)
}
const runMain = () => { const s = editor.value?.selected() ?? ''; void run(s ? 'selection' : 'current', s) }

// ---- Explain (S39)
function explainNow(analyze: boolean) {
  const sel = editor.value?.selected() ?? ''
  void store.explain(q.value, analyze, sel)
}
function openInEditor(sqlText: string) { store.newQuery(sqlText, 'Draft') }

// ---- Limit and timeout: plain numbers, kept in range
const limitText = ref(String(q.value.limit))
const timeoutText = ref(String(q.value.timeoutS))
watch(limitText, (v) => { const n = Math.floor(Number(v)); if (Number.isFinite(n) && n >= 1) q.value.limit = Math.min(n, 1_000_000) })
watch(timeoutText, (v) => { const n = Math.floor(Number(v)); if (Number.isFinite(n) && n >= 1) q.value.timeoutS = Math.min(n, 3600) })

// ---- Save (⌘S)
const saving = ref(false)
const saveName = ref('')
const thisOnly = ref(false)
const saveError = ref('')
async function save(force = false) {
  saveError.value = ''
  // A query that came from the Saved list is saved over itself; a new one asks for a name first.
  if (q.value.savedId && !force) {
    try { await store.save(q.value, q.value.title, false); await databases.refreshSaved() }
    catch (e) { saveError.value = (e as { message?: string }).message ?? 'Couldn\'t save it.' }
    return
  }
  if (!saving.value) { saving.value = true; saveName.value = q.value.title; return }
  if (!saveName.value.trim()) { saveError.value = 'Give the query a name.'; return }
  try {
    await store.save(q.value, saveName.value, thisOnly.value)
    await databases.refreshSaved()
    saving.value = false
  }
  catch (e) { saveError.value = (e as { message?: string }).message ?? 'Couldn\'t save it.' }
}

// ---- The strip: results, messages, history, saved
function show(v: QueryView) {
  q.value.view = v
  if (v === 'history') void store.loadHistory()
  if (v === 'saved' && !databases.savedLoaded) void databases.refreshSaved()
}
const copied = ref('')
async function copy(f: CopyFormat) {
  const r = result.value
  if (!r) return
  await navigator.clipboard.writeText(copyAs(f, r.cols.map(c => c.name), r.rows))
  copied.value = COPY_FORMATS.find(x => x.id === f)!.label
  setTimeout(() => { copied.value = '' }, 1600)
}
function useText(text: string) { q.value.text = text; nextTick(() => editor.value?.focus()) }
function openSaved(s: { name: string, body: string, id: string }) {
  if (!q.value.text.trim() && !q.value.savedId) { q.value.text = s.body; q.value.title = s.name; q.value.savedId = s.id; q.value.saved = s.body }
  else store.newQuery(s.body, s.name, s.id)
}
const cellText = computed(() => {
  const r = result.value
  const s = r?.selected
  const c = s ? r?.rows[s.row]?.[s.col] : null
  if (!c || c.v === null) return null
  if (c.kind === 'json') { try { return JSON.stringify(JSON.parse(c.v), null, 2) } catch { return c.v } }
  return c.v
})
// ⌘S saves from anywhere in the tab; the editor handles its own (and marks the event as handled).
function onKey(e: KeyboardEvent) {
  if (!e.defaultPrevented && (e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 's') { e.preventDefault(); void save() }
}
// ---- The transaction bar: its age is read from the server every 20 s; a quiet two minutes turns it amber.
const idle = computed(() => (q.value.txn?.age_s ?? 0) >= 120)
const ageText = computed(() => { const a = q.value.txn?.age_s ?? 0; return a < 60 ? 'started just now' : `started ${Math.floor(a / 60)} min ago` })
let poll: ReturnType<typeof setInterval> | undefined
watch(() => !!q.value.txn?.active, (on) => {
  clearInterval(poll)
  if (on) poll = setInterval(() => { if (!q.value.running) void store.txn(q.value, 'state') }, 20_000)
}, { immediate: true })
onBeforeUnmount(() => clearInterval(poll))
onMounted(() => { if (!databases.savedLoaded) void databases.refreshSaved() })
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col" :data-testid="tid(`sql-query-${q.id}`)" @keydown="onKey">
    <div class="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3" role="toolbar" aria-label="Query">
      <Button v-if="!q.running" size="sm" :data-testid="tid('sql-run')" @click="runMain"><Play />Run<kbd class="ml-1 font-mono text-2xs opacity-70">⌘↵</kbd></Button>
      <Button v-else size="sm" variant="destructive" :data-testid="tid('sql-stop')" @click="store.stop(q)"><Square />Stop</Button>
      <Button variant="secondary" size="sm" :disabled="q.running" @click="runSelection">Run selection</Button>
      <Button variant="secondary" size="sm" :disabled="q.running" @click="run('all')">Run all</Button>
      <Button v-if="!store.isMy" variant="secondary" size="sm" :disabled="q.running" :data-testid="tid('sql-explain')" @click="explainNow(false)">Explain</Button>
      <Button v-if="!store.isMy" variant="ghost" size="sm" :disabled="q.running" :data-testid="tid('sql-explain-analyze')" title="Runs the statement (a read) inside a transaction that is rolled back, and times each step" @click="explainNow(true)">Explain analyze</Button>
      <Button variant="ghost" size="sm" @click="store.format(q)">Format</Button>
      <Button variant="ghost" size="sm" @click="save()"><Save />Save</Button>
      <Button variant="secondary" size="sm" :aria-pressed="q.manual" :disabled="q.running || !!q.txn?.active" :title="q.manual ? 'Statements wait for Commit' : 'Each statement commits on its own'" :data-testid="tid('sql-manual')" @click="store.setManual(q, !q.manual)">{{ q.manual ? 'Manual' : 'Auto-commit' }}</Button>
      <span class="ml-auto flex items-center gap-2 text-xs text-muted">
        <label :for="`${tabKey}-${q.id}-limit`">Limit</label>
        <Input :id="`${tabKey}-${q.id}-limit`" v-model="limitText" class="h-7 w-20 text-xs" inputmode="numeric" autocomplete="off" />
        <label :for="`${tabKey}-${q.id}-timeout`">Timeout</label>
        <Input :id="`${tabKey}-${q.id}-timeout`" v-model="timeoutText" class="h-7 w-14 text-xs" inputmode="numeric" autocomplete="off" /><span aria-hidden="true">s</span>
      </span>
    </div>
    <div v-if="q.txn?.active" :class="cn('flex h-11 shrink-0 items-center gap-3 border-b px-3 text-sm', idle ? 'border-warning/40 bg-warning/10' : 'border-border bg-surface')" role="status" :data-testid="tid('sql-txn-bar')">
      <TriangleAlert class="size-4 shrink-0 text-warning" aria-hidden="true" />
      <span class="min-w-0 flex-1 truncate">
        <b class="font-medium">{{ q.txn.failed ? 'The transaction failed' : 'In a transaction' }}</b>
        · {{ q.txn.failed ? 'roll back to carry on' : `${q.txn.statements} ${q.txn.statements === 1 ? 'statement' : 'statements'} not committed` }}
        · {{ ageText }}<template v-if="q.txn.locks"> · holds {{ q.txn.locks }} {{ q.txn.locks === 1 ? 'lock' : 'locks' }}</template>
        <template v-if="idle"> · still open: others may be waiting on its locks</template>
      </span>
      <Button variant="secondary" size="sm" :disabled="q.running" @click="store.txn(q, 'rollback')">Roll back</Button>
      <Button size="sm" :disabled="q.running || q.txn.failed" @click="store.txn(q, 'commit')">Commit</Button>
    </div>
    <SqlConfirmDialog :tab-key="tabKey" :query="q" />
    <form v-if="saving" class="flex h-11 shrink-0 items-center gap-2 border-b border-border bg-surface px-3" @submit.prevent="save(true)" @keydown.esc="saving = false">
      <label :for="`${tabKey}-${q.id}-name`" class="text-xs text-muted">Name</label>
      <Input :id="`${tabKey}-${q.id}-name`" v-model="saveName" class="h-7 w-64 text-xs" autocomplete="off" data-autofocus :aria-invalid="!!saveError" />
      <label class="flex items-center gap-1.5 text-xs text-muted"><input v-model="thisOnly" type="checkbox" class="accent-[var(--accent)]">Only this database</label>
      <Button type="submit" size="sm">Save</Button>
      <Button type="button" variant="ghost" size="sm" @click="saving = false">Cancel</Button>
      <span v-if="saveError" role="alert" class="text-xs text-danger">{{ saveError }}</span>
    </form>
    <p v-else-if="saveError" role="alert" class="shrink-0 border-b border-border px-3 py-1.5 text-xs text-danger">{{ saveError }}</p>

    <div class="flex h-[40%] min-h-24 shrink-0 flex-col border-b border-border">
      <SqlEditor
        ref="editor" v-model="q.text" :catalog="store.catalog" :label="`${q.title} SQL`" @cursor="(p: number) => { q.cursor = p }"
        @run="(m: 'current' | 'selection' | 'all', s: string) => run(m, s)" @save="save()" @format="store.format(q)"
      />
    </div>

    <div class="flex h-[34px] shrink-0 items-stretch overflow-x-auto border-b border-border" role="group" aria-label="Results">
      <button
        v-for="(r, i) in q.results" :key="r.index" type="button" :aria-current="q.view === `r${i}` ? 'true' : undefined"
        :class="cn('flex shrink-0 cursor-default items-center gap-2 border-r border-border px-3 text-xs text-muted outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent', q.view === `r${i}` && 'bg-bg text-text shadow-[inset_0_2px_0_var(--color-accent)]')"
        @click="show(`r${i}`)"
      >
        <span v-if="r.state === 'failed'" class="size-1.5 rounded-full bg-danger" aria-label="failed" />
        Result {{ i + 1 }}
        <span class="text-muted">· {{ r.state === 'running' ? `${fmt(r.rows.length)} rows…` : r.state === 'failed' ? 'failed' : `${fmt(r.rows.length)} ${r.rows.length === 1 ? 'row' : 'rows'}${r.elapsedMs !== null ? ` · ${r.elapsedMs} ms` : ''}` }}</span>
      </button>
      <button
        v-for="v in (q.plan ? (['messages', 'history', 'saved', 'explain'] as const) : (['messages', 'history', 'saved'] as const))" :key="v" type="button" :aria-current="q.view === v ? 'true' : undefined"
        :class="cn('flex shrink-0 cursor-default items-center gap-1.5 border-r border-border px-3 text-xs capitalize text-muted outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent', q.view === v && 'bg-bg text-text shadow-[inset_0_2px_0_var(--color-accent)]')"
        @click="show(v)"
      >
        <History v-if="v === 'history'" class="size-3.5" aria-hidden="true" /><Star v-else-if="v === 'saved'" class="size-3.5" aria-hidden="true" />{{ v }}
        <span v-if="v === 'messages' && q.messages.some(m => m.kind !== 'info')" class="size-1.5 rounded-full bg-danger" aria-label="has errors" />
      </button>
      <span class="ml-auto flex shrink-0 items-center gap-2 px-2">
        <span v-if="copied" class="flex items-center gap-1 text-xs text-success" role="status"><Check class="size-3.5" />Copied as {{ copied }}</span>
        <DropdownMenu>
          <DropdownMenuTrigger as-child><Button variant="ghost" size="xs" :disabled="!result || !result.cols.length"><Copy />Copy as…</Button></DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem v-for="f in COPY_FORMATS" :key="f.id" @select="copy(f.id)">{{ f.label }}</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </span>
    </div>

    <div class="flex min-h-0 flex-1">
      <template v-if="result">
        <div class="flex min-w-0 flex-1 flex-col">
          <DataGrid
            v-if="result.cols.length" :label="`Result ${result.index + 1}`" :columns="result.cols" :rows="result.rows" :selected="result.selected"
            @select="(r: number, c: number) => { result!.selected = { row: r, col: c } }" @sort="() => {}"
          />
          <p v-else class="p-4 text-sm text-muted">{{ result.state === 'running' ? 'Running…' : 'This statement returned no rows.' }}</p>
          <p v-if="result.state === 'running'" class="shrink-0 border-t border-border px-3 py-1.5 text-xs text-muted" role="status">Running… {{ fmt(result.rows.length) }} rows so far.</p>
          <p v-else-if="result.more" class="shrink-0 border-t border-border px-3 py-1.5 text-xs text-muted" role="status" :data-testid="tid('sql-more')">First {{ fmt(q.limit) }} rows. Raise the limit to see more.</p>
          <p v-else-if="result.cols.length && !result.rows.length && result.state === 'done'" class="shrink-0 border-t border-border px-3 py-1.5 text-xs text-muted">No rows.</p>
        </div>
        <aside v-if="result.selected && cellText !== null" class="w-72 shrink-0 overflow-auto border-l border-border p-3 text-sm" aria-label="Cell">
          <h3 class="truncate font-mono text-xs font-medium">{{ result.cols[result.selected.col]?.name }}</h3>
          <p class="mb-2 text-xs text-muted">{{ result.cols[result.selected.col]?.type_name }} · row {{ result.selected.row + 1 }}</p>
          <pre class="max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-md border border-border bg-surface p-2 font-mono text-xs">{{ cellText }}</pre>
        </aside>
      </template>

      <div v-else-if="q.view === 'messages'" class="min-w-0 flex-1 overflow-auto p-3 text-sm" aria-live="polite" :data-testid="tid('sql-messages')">
        <p v-if="!q.messages.length && !q.running" class="text-muted">Run a query to see rows here. ⌘↵ runs the statement your cursor is in.</p>
        <p v-if="q.running && !q.messages.length" class="text-muted" aria-busy="true">Running…</p>
        <div v-for="(m, i) in q.messages" :key="i" :class="cn('mb-2 rounded-md border px-3 py-2', m.kind === 'info' ? 'border-border' : 'border-danger/40')" :role="m.kind === 'info' ? undefined : 'alert'">
          <pre v-if="m.statement" class="mb-1 max-h-16 overflow-hidden truncate whitespace-pre-wrap font-mono text-xs text-muted">{{ m.statement }}</pre>
          <p :class="m.kind === 'info' ? 'text-text' : 'text-danger'">{{ m.text }}</p>
        </div>
        <p v-if="q.totalMs !== null && q.messages.length > 1" class="text-xs text-muted">Total {{ q.totalMs }} ms</p>
      </div>

      <SqlPlanView v-else-if="q.view === 'explain' && q.plan" :plan="q.plan" @open="openInEditor" @rerun="(a: boolean) => explainNow(a)" />

      <div v-else-if="q.view === 'history'" class="min-w-0 flex-1 overflow-auto p-2 text-sm">
        <div v-if="store.history.length" class="mb-1 flex justify-end"><Button variant="ghost" size="xs" @click="store.clearHistory()"><Trash2 />Clear history</Button></div>
        <p v-if="!store.history.length" class="p-2 text-muted">Nothing run on this database yet.</p>
        <button v-for="(h, i) in store.history" :key="i" type="button" class="block w-full cursor-default truncate rounded-md px-2 py-1.5 text-left font-mono text-xs hover:bg-surface focus-visible:ring-2 focus-visible:ring-accent" :title="h" @click="useText(h)">{{ h.replace(/\s+/g, ' ') }}</button>
      </div>

      <div v-else class="min-w-0 flex-1 overflow-auto p-2 text-sm">
        <p v-if="!saved.length" class="p-2 text-muted">Save a query with ⌘S and it shows up here for every database of that engine.</p>
        <button v-for="s in saved" :key="s.id" type="button" class="flex w-full cursor-default items-center gap-3 rounded-md px-2 py-1.5 text-left hover:bg-surface focus-visible:ring-2 focus-visible:ring-accent" @click="openSaved(s)">
          <span class="font-medium">{{ s.name }}</span><span class="truncate font-mono text-xs text-muted">{{ s.body.replace(/\s+/g, ' ') }}</span>
        </button>
      </div>
    </div>
  </div>
</template>
