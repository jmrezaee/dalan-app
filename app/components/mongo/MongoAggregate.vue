<script setup lang="ts">
// S45 "MongoAggregate": the pipeline as a list of stages that can be switched off, the output after the chosen stage,
// a hint, the pipeline as code, and a view made from it. A pipeline that ends in $out or $merge is a write: it is
// held and confirmed like any other change.
import { ArrowDown, ArrowUp, Play, Plus, Trash2 } from '@lucide/vue'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { cn } from '@/lib/utils'
import { STAGE_OPS, type CollTab } from '~/stores/mongo'

const props = defineProps<{ tabKey: string, tabData: CollTab }>()
const store = useMongoStore(props.tabKey)
const tab = props.tabData
const a = tab.agg
const LANGS = [{ id: 'mongosh', label: 'mongosh' }, { id: 'node', label: 'Node.js' }, { id: 'python', label: 'Python' }, { id: 'json', label: 'JSON' }] as const
const newOp = ref<string>('$match')
const writes = computed(() => a.stages.some(s => s.enabled && (s.op === '$out' || s.op === '$merge')))
const copied = ref(false)
async function copyCode() {
  if (!a.code) return
  try { await navigator.clipboard.writeText(a.code.text); copied.value = true; setTimeout(() => { copied.value = false }, 1500) }
  catch { /* the text stays selectable */ }
}
function pick(i: number) {
  a.selected = i
  void store.aggRun(tab)
}
</script>

<template>
  <div class="flex min-h-0 flex-1" :data-testid="tid('mongo-aggregate')">
    <section class="flex w-[420px] shrink-0 flex-col border-r border-border" aria-label="Pipeline">
      <ol class="m-0 min-h-0 flex-1 list-none overflow-auto p-2">
        <li v-for="(s, i) in a.stages" :key="s.id" :class="cn('mb-2 rounded-md border bg-surface', a.selected === i ? 'border-accent' : 'border-border', !s.enabled && 'opacity-60')" :data-testid="tid(`mongo-stage-${i}`)">
          <div class="flex items-center gap-1.5 border-b border-border px-2 py-1">
            <input v-model="s.enabled" type="checkbox" class="size-3.5 accent-[var(--color-accent)]" :aria-label="`Stage ${i + 1} ${s.op} on`">
            <button type="button" class="flex-1 cursor-default text-left font-mono text-xs font-medium outline-none focus-visible:ring-2 focus-visible:ring-accent" :aria-pressed="a.selected === i" @click="pick(i)">{{ i + 1 }}. {{ s.op }}</button>
            <Button type="button" size="icon-sm" variant="ghost" :disabled="i === 0" :aria-label="`Move stage ${i + 1} up`" @click="store.aggMove(tab, i, -1)"><ArrowUp class="size-3.5" aria-hidden="true" /></Button>
            <Button type="button" size="icon-sm" variant="ghost" :disabled="i === a.stages.length - 1" :aria-label="`Move stage ${i + 1} down`" @click="store.aggMove(tab, i, 1)"><ArrowDown class="size-3.5" aria-hidden="true" /></Button>
            <Button type="button" size="icon-sm" variant="ghost" class="text-danger" :aria-label="`Remove stage ${i + 1}`" @click="store.aggRemove(tab, i)"><Trash2 class="size-3.5" aria-hidden="true" /></Button>
          </div>
          <Textarea v-model="s.body" class="min-h-16 rounded-none border-0 bg-transparent font-mono text-xs" spellcheck="false" :aria-label="`Stage ${i + 1} ${s.op} body`" />
        </li>
        <li v-if="!a.stages.length" class="p-3 text-sm text-muted">No stages. Add one below.</li>
      </ol>
      <div class="flex shrink-0 items-center gap-2 border-t border-border p-2">
        <select v-model="newOp" class="h-8 flex-1 rounded-md border border-border-input bg-surface px-2 font-mono text-xs" aria-label="Stage to add"><option v-for="o in STAGE_OPS" :key="o" :value="o">{{ o }}</option></select>
        <Button type="button" size="sm" variant="secondary" :data-testid="tid('mongo-add-stage')" @click="store.aggAdd(tab, newOp)"><Plus class="size-3.5" aria-hidden="true" />Add stage</Button>
      </div>
    </section>

    <section class="flex min-w-0 flex-1 flex-col" aria-label="Output">
      <div class="flex shrink-0 flex-wrap items-end gap-2 border-b border-border p-2">
        <label class="flex flex-col gap-0.5 text-2xs text-muted">Hint (index name or keys)
          <Input v-model="a.hint" class="h-7 w-56 font-mono text-xs text-text" placeholder="status_1" autocomplete="off" spellcheck="false" aria-label="Hint" />
        </label>
        <label class="flex flex-col gap-0.5 text-2xs text-muted">Output documents
          <select v-model.number="a.sample" class="h-7 rounded-md border border-border-input bg-surface px-1.5 text-xs text-text" aria-label="Output documents"><option v-for="n in [10, 20, 50, 100]" :key="n" :value="n">{{ n }}</option></select>
        </label>
        <Button type="button" size="sm" :disabled="a.loading || !a.stages.length" :data-testid="tid('mongo-agg-run')" @click="store.aggRun(tab)"><Play class="size-3.5" aria-hidden="true" />Run to stage {{ a.selected + 1 }}</Button>
        <span class="ml-auto" />
        <Button v-if="writes" type="button" size="sm" variant="secondary" :data-testid="tid('mongo-agg-write')" @click="store.aggWrite(tab)">Run with $out / $merge…</Button>
      </div>
      <p v-if="a.error" role="alert" class="shrink-0 border-b border-border bg-surface px-3 py-2 text-xs text-danger" :data-testid="tid('mongo-agg-error')">{{ a.error }}</p>
      <div class="min-h-0 flex-1 overflow-auto" tabindex="0" role="region" aria-label="Stage output" :aria-busy="a.loading">
        <p v-if="!a.result && !a.error" class="p-4 text-sm text-muted">Pick a stage and run to see its output.</p>
        <template v-else-if="a.result">
          <p class="px-3 pt-2 text-xs text-muted" :data-testid="tid('mongo-agg-summary')">{{ a.result.docs.length }}{{ a.result.more ? '+' : '' }} documents · {{ a.result.took_ms }} ms<template v-if="a.result.stopped_at_write"> · stopped before $out / $merge: nothing was written</template></p>
          <ol class="m-0 flex list-none flex-col gap-2 p-3" aria-label="Output documents">
            <li v-for="(d, i) in a.result.docs" :key="i" class="rounded-md border border-border bg-surface"><pre class="m-0 max-h-60 overflow-auto whitespace-pre-wrap break-all p-2 font-mono text-xs">{{ d.relaxed }}</pre></li>
          </ol>
        </template>
      </div>
      <div class="shrink-0 border-t border-border p-2">
        <div class="flex flex-wrap items-center gap-2">
          <span class="text-xs text-muted">Export as</span>
          <Button v-for="l in LANGS" :key="l.id" type="button" size="sm" :variant="a.code?.language === l.id ? 'secondary' : 'ghost'" @click="store.aggCode(tab, l.id)">{{ l.label }}</Button>
          <span class="ml-auto" />
          <Input v-model="a.viewName" class="h-7 w-40 text-xs" placeholder="View name" aria-label="View name" autocomplete="off" spellcheck="false" />
          <Button type="button" size="sm" variant="secondary" :disabled="!a.viewName.trim()" :data-testid="tid('mongo-create-view')" @click="store.aggView(tab)">Create view…</Button>
        </div>
        <div v-if="a.code" class="mt-2">
          <pre class="m-0 max-h-48 overflow-auto rounded-md border border-border bg-bg p-2 font-mono text-xs" aria-label="Pipeline code" :data-testid="tid('mongo-agg-code')">{{ a.code.text }}</pre>
          <Button type="button" size="sm" variant="ghost" class="mt-1" @click="copyCode">{{ copied ? 'Copied' : 'Copy' }}</Button>
        </div>
      </div>
    </section>
  </div>
</template>
