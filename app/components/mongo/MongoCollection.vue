<script setup lang="ts">
// S44 "MongoDocs": one collection. The query bar (filter, projection, sort, collation, limit), the list, table and
// JSON views, the pager, and the document editor. Changes are held by Rust and shown here before they are sent.
import { ChevronLeft, ChevronRight, Pencil, Plus, RefreshCw, Trash2 } from '@lucide/vue'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'
import { PAGE_LIMITS, type CollTab } from '~/stores/mongo'

const props = defineProps<{ tabKey: string, tabData: CollTab }>()
const store = useMongoStore(props.tabKey)
// The collection's state lives in the store; this is the same reactive object, edited through the form fields.
const tab = props.tabData
const t = computed(() => props.tabData)

const VIEWS = [{ id: 'list', label: 'List' }, { id: 'table', label: 'Table' }, { id: 'json', label: 'JSON' }] as const
const first = computed(() => t.value.page * t.value.limit + 1)
const range = computed(() => (t.value.docs.length ? `${first.value.toLocaleString('en-US')}–${(first.value + t.value.docs.length - 1).toLocaleString('en-US')}` : '0'))
const countText = computed(() => {
  const c = t.value.count
  return c ? `${c.estimate ? '≈ ' : ''}${Math.round(c.n ?? 0).toLocaleString('en-US')} documents${c.estimate ? ' (estimate)' : ''}` : ''
})
const isView = computed(() => t.value.kind === 'view')

/** The table view's columns: the top-level fields of the page, in first-seen order. */
// A cut document is not valid JSON: its cells say so instead of showing nothing.
const parsed = computed(() => t.value.docs.map((d) => { try { return d.cut ? {} : JSON.parse(d.relaxed) as Record<string, unknown> } catch { return {} } }))
const cutCell = (i: number, c: string) => (t.value.docs[i]?.cut && c === columns.value[0] ? 'Too long to show here. Edit opens it.' : cell(parsed.value[i]?.[c]))
/** Projected results aren't whole documents: saving one would drop the other fields. */
const projected = computed(() => t.value.applied.project.trim() !== '')
const columns = computed(() => {
  const seen = new Set<string>()
  for (const p of parsed.value) for (const k of Object.keys(p)) seen.add(k)
  return [...seen].slice(0, 12)
})
const cell = (v: unknown) => (v === undefined ? '' : v === null ? 'null' : typeof v === 'object' ? JSON.stringify(v) : String(v))
const allJson = computed(() => `[\n${t.value.docs.map(d => d.relaxed).join(',\n')}\n]`)

/** Live JSON check of the editor's text, so a typo is named before Save. */
const editorProblem = computed(() => {
  const e = t.value.editor
  if (!e) return ''
  try { JSON.parse(e.text); return '' } catch (x) { return (x as Error).message }
})
// Esc or a click outside keeps unsaved typing; Cancel is the explicit way out.
const editorOpen = computed({ get: () => t.value.editor !== null, set: (v: boolean) => { if (!v && t.value.editor && t.value.editor.text === t.value.editor.original) t.value.editor = null } })
const reviewOpen = computed({ get: () => t.value.review !== null, set: (v: boolean) => { if (!v) store.cancelReview(t.value) } })
const showSample = ref(false)
watch(() => t.value.review?.pending.id, () => { showSample.value = false })
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col" :data-testid="tid(`mongo-collection-${tab.coll}`)">
    <div role="group" aria-label="Section" class="flex h-9 shrink-0 items-center gap-1 border-b border-border px-2">
      <Button type="button" size="sm" :variant="tab.section === 'documents' ? 'secondary' : 'ghost'" :class="tab.section === 'documents' ? '' : 'text-muted'" :aria-pressed="tab.section === 'documents'" @click="tab.section = 'documents'">Documents</Button>
      <Button type="button" size="sm" :variant="tab.section === 'aggregate' ? 'secondary' : 'ghost'" :class="tab.section === 'aggregate' ? '' : 'text-muted'" :aria-pressed="tab.section === 'aggregate'" :data-testid="tid('mongo-section-aggregate')" @click="tab.section = 'aggregate'">Aggregate</Button>
      <Button type="button" size="sm" :variant="tab.section === 'schema' ? 'secondary' : 'ghost'" :class="tab.section === 'schema' ? '' : 'text-muted'" :aria-pressed="tab.section === 'schema'" :data-testid="tid('mongo-section-schema')" @click="store.openSchema(tab)">Schema</Button>
    </div>
    <MongoAggregate v-if="tab.section === 'aggregate'" :tab-key="tabKey" :tab-data="tab" />
    <MongoSchema v-else-if="tab.section === 'schema'" :tab-key="tabKey" :tab-data="tab" />
    <template v-if="tab.section === 'documents'">
    <form class="grid shrink-0 grid-cols-[1fr_1fr] gap-2 border-b border-border p-2 lg:grid-cols-[2fr_1fr_1fr_1fr_auto]" aria-label="Query" @submit.prevent="store.apply(tab)">
      <label class="flex flex-col gap-0.5 text-2xs text-muted">Filter
        <Input v-model="tab.filter" class="h-7 font-mono text-xs text-text" placeholder="{ &quot;status&quot;: &quot;open&quot; }" autocomplete="off" spellcheck="false" aria-label="Filter" :data-testid="tid('mongo-filter')" />
      </label>
      <label class="flex flex-col gap-0.5 text-2xs text-muted">Project
        <Input v-model="tab.project" class="h-7 font-mono text-xs text-text" placeholder="{ &quot;name&quot;: 1 }" autocomplete="off" spellcheck="false" aria-label="Project" />
      </label>
      <label class="flex flex-col gap-0.5 text-2xs text-muted">Sort
        <Input v-model="tab.sort" class="h-7 font-mono text-xs text-text" placeholder="{ &quot;_id&quot;: -1 }" autocomplete="off" spellcheck="false" aria-label="Sort" />
      </label>
      <label class="flex flex-col gap-0.5 text-2xs text-muted">Collation
        <Input v-model="tab.collation" class="h-7 font-mono text-xs text-text" placeholder="{ &quot;locale&quot;: &quot;en&quot; }" autocomplete="off" spellcheck="false" aria-label="Collation" />
      </label>
      <div class="flex items-end gap-1.5">
        <Button type="submit" size="sm" :disabled="tab.loading" :data-testid="tid('mongo-apply')">Apply</Button>
        <Button type="button" size="icon-sm" variant="ghost" aria-label="Refresh" :disabled="tab.loading" @click="store.load(tab)"><RefreshCw class="size-3.5" aria-hidden="true" /></Button>
      </div>
    </form>

    <div class="flex h-9 shrink-0 items-center gap-3 border-b border-border px-3 text-xs">
      <div role="group" aria-label="View" class="flex gap-1">
        <Button v-for="v in VIEWS" :key="v.id" type="button" size="sm" :variant="tab.view === v.id ? 'secondary' : 'ghost'" :class="tab.view === v.id ? '' : 'text-muted'" :aria-pressed="tab.view === v.id" @click="tab.view = v.id">{{ v.label }}</Button>
      </div>
      <button type="button" class="cursor-default text-muted underline-offset-2 hover:text-text hover:underline" :data-testid="tid('mongo-count')" @click="store.countDocs(tab)">{{ countText || 'Count' }}</button>
      <span v-if="tab.tookMs !== null" class="text-muted">{{ tab.tookMs }} ms</span>
      <span v-if="projected" class="text-muted" data-part="projected">Projection on: these are parts of documents, so they can't be edited.</span>
      <span class="ml-auto" />
      <label class="flex items-center gap-1.5 text-muted">Per page
        <select class="h-7 rounded-md border border-border-input bg-surface px-1.5 text-xs text-text" :value="tab.limit" aria-label="Documents per page" @change="store.setLimit(tab, Number(($event.target as HTMLSelectElement).value))"><option v-for="n in PAGE_LIMITS" :key="n" :value="n">{{ n }}</option></select>
      </label>
      <Button v-if="!isView" type="button" size="sm" variant="secondary" :data-testid="tid('mongo-new-doc')" @click="store.startNew(tab)"><Plus class="size-3.5" aria-hidden="true" />New document</Button>
    </div>

    <p v-if="tab.error" role="alert" class="shrink-0 border-b border-border bg-surface px-3 py-2 text-xs text-danger" :data-testid="tid('mongo-error')">{{ tab.error }}</p>

    <div class="min-h-0 flex-1 overflow-auto" :aria-busy="tab.loading">
      <p v-if="tab.loading && !tab.docs.length" class="p-4 text-sm text-muted">Reading documents…</p>
      <p v-else-if="!tab.docs.length && !tab.error" class="p-4 text-sm text-muted">{{ tab.applied.filter ? 'No document matches this filter.' : 'This collection has no documents yet.' }}</p>

      <ol v-else-if="tab.view === 'list'" class="m-0 flex list-none flex-col gap-2 p-3" aria-label="Documents">
        <li v-for="(d, i) in tab.docs" :key="`${i}-${d.id}`" class="rounded-md border border-border bg-surface" :data-testid="tid('mongo-doc')">
          <div class="flex items-center gap-2 border-b border-border px-2 py-1 text-2xs text-muted">
            <span class="truncate font-mono">_id {{ parsed[i]?._id !== undefined ? cell(parsed[i]?._id) : '' }}</span>
            <span class="ml-auto" />
            <template v-if="!isView && !projected">
              <Button type="button" size="sm" variant="ghost" :aria-label="`Edit document ${i + 1}`" @click="store.startEdit(tab, d)"><Pencil class="size-3.5" aria-hidden="true" />Edit</Button>
              <Button type="button" size="sm" variant="ghost" class="text-danger" :aria-label="`Delete document ${i + 1}`" @click="store.deleteDoc(tab, d)"><Trash2 class="size-3.5" aria-hidden="true" />Delete</Button>
            </template>
          </div>
          <pre class="m-0 max-h-72 overflow-auto whitespace-pre-wrap break-all p-2 font-mono text-xs">{{ d.relaxed }}</pre>
          <p v-if="d.cut" class="px-2 pb-2 text-2xs text-muted">This document is long, so it is cut here. Edit shows all of it.</p>
        </li>
      </ol>

      <table v-else-if="tab.view === 'table'" class="w-max min-w-full border-collapse text-xs" aria-label="Documents as a table">
        <thead class="sticky top-0 bg-surface">
          <tr><th v-for="c in columns" :key="c" scope="col" class="h-7 whitespace-nowrap border-b border-border px-2 text-left font-medium">{{ c }}</th><th scope="col" class="w-px border-b border-border px-2"><span class="sr-only">Actions</span></th></tr>
        </thead>
        <tbody>
          <tr v-for="(d, i) in tab.docs" :key="`${i}-${d.id}`" :class="cn('hover:bg-surface', tab.selected === i && 'bg-surface-hover')" @click="tab.selected = i" @dblclick="!isView && !projected && store.startEdit(tab, d)">
            <td v-for="c in columns" :key="c" class="max-w-[260px] truncate whitespace-nowrap border-b border-border px-2 py-1 font-mono">{{ cutCell(i, c) }}</td>
            <td class="whitespace-nowrap border-b border-border px-1">
              <template v-if="!isView && !projected">
                <Button type="button" size="icon-sm" variant="ghost" :aria-label="`Edit document ${i + 1}`" @click.stop="store.startEdit(tab, d)"><Pencil class="size-3.5" aria-hidden="true" /></Button>
                <Button type="button" size="icon-sm" variant="ghost" class="text-danger" :aria-label="`Delete document ${i + 1}`" @click.stop="store.deleteDoc(tab, d)"><Trash2 class="size-3.5" aria-hidden="true" /></Button>
              </template>
            </td>
          </tr>
        </tbody>
      </table>

      <pre v-else class="m-0 whitespace-pre-wrap break-all p-3 font-mono text-xs" aria-label="Documents as JSON">{{ allJson }}</pre>
    </div>

    <div class="flex h-9 shrink-0 items-center gap-2 border-t border-border px-3 text-xs text-muted">
      <Button type="button" size="icon-sm" variant="ghost" aria-label="Previous page" :disabled="tab.page === 0 || tab.loading" @click="store.prevPage(tab)"><ChevronLeft class="size-4" aria-hidden="true" /></Button>
      <span :data-testid="tid('mongo-range')">Documents {{ range }}</span>
      <Button type="button" size="icon-sm" variant="ghost" aria-label="Next page" :disabled="!tab.more || tab.loading" @click="store.nextPage(tab)"><ChevronRight class="size-4" aria-hidden="true" /></Button>
    </div>

    </template>

    <Dialog v-model:open="editorOpen">
      <DialogContent v-if="tab.editor" class="flex h-[80vh] flex-col gap-3 sm:max-w-[720px]" :data-testid="tid('mongo-editor-dialog')">
        <DialogHeader>
          <DialogTitle>{{ tab.editor.mode === 'new' ? `New document in ${tab.coll}` : `Edit document in ${tab.coll}` }}</DialogTitle>
          <DialogDescription>Extended JSON: numbers, dates and ids keep their exact types, like <code class="font-mono">{ "$numberLong": "5" }</code>. Nothing is sent until you confirm.</DialogDescription>
        </DialogHeader>
        <div class="min-h-0 flex-1"><MongoJsonEditor v-model="tab.editor.text" :label="tab.editor.mode === 'new' ? 'New document' : 'Document'" @save="!editorProblem && store.saveEditor(tab)" /></div>
        <p v-if="editorProblem" class="text-xs text-warning" data-part="problem">{{ editorProblem }}</p>
        <p v-if="tab.editor.error" role="alert" class="text-xs text-danger" :data-testid="tid('mongo-editor-error')">{{ tab.editor.error }}</p>
        <DialogFooter>
          <Button variant="secondary" @click="tab.editor = null">Cancel</Button>
          <Button :disabled="!!editorProblem" :data-testid="tid('mongo-editor-save')" @click="store.saveEditor(tab)">Review change</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>

    <AlertDialog v-model:open="reviewOpen">
      <AlertDialogContent v-if="tab.review" class="gap-3 sm:max-w-[560px]" :data-testid="tid('mongo-review')">
        <AlertDialogHeader>
          <AlertDialogTitle>{{ tab.review.pending.summary }}</AlertDialogTitle>
          <AlertDialogDescription>
            Nothing is sent until you confirm.
            <span v-if="tab.review.pending.environment === 'production'" class="text-warning">{{ tab.review.pending.db_name }} is a production database.</span>
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div v-if="tab.review.pending.sample">
          <button type="button" class="cursor-default text-xs text-muted underline underline-offset-2 hover:text-text" :aria-expanded="showSample" @click="showSample = !showSample">{{ showSample ? 'Hide' : 'Show' }} the document as it is now</button>
          <pre v-if="showSample" class="mt-1 max-h-48 overflow-auto rounded-md border border-border bg-bg p-2 font-mono text-xs">{{ tab.review.pending.sample }}</pre>
        </div>
        <label v-if="tab.review.pending.level === 'typed'" class="flex flex-col gap-1 text-xs text-muted">Type {{ tab.review.pending.db_name }} to confirm
          <Input v-model="tab.review.typed" class="font-mono text-xs text-text" autocomplete="off" spellcheck="false" :data-testid="tid('mongo-typed')" @keydown.enter.prevent="store.commitReview(tab)" />
        </label>
        <p v-if="tab.review.error" role="alert" class="text-xs text-danger">{{ tab.review.error }}</p>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <Button :disabled="tab.review.busy || (tab.review.pending.level === 'typed' && tab.review.typed !== tab.review.pending.db_name)" :data-testid="tid('mongo-confirm')" @click="store.commitReview(tab)">Confirm</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
