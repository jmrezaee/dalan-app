<script setup lang="ts">
// S46 "MongoSchema": a sample of the collection by field and type, its indexes with how often they are used (and
// create or drop), and its validation rules with a count of the documents that would break them.
import { Trash2 } from '@lucide/vue'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import type { CollTab } from '~/stores/mongo'

const props = defineProps<{ tabKey: string, tabData: CollTab }>()
const store = useMongoStore(props.tabKey)
const tab = props.tabData
const s = tab.sch
const pct = (n: number) => (s.schema?.sampled ? Math.round((n / s.schema.sampled) * 100) : 0)
const num = (n: number | null) => (n === null ? 'n/a' : Math.round(n).toLocaleString('en-US'))
const since = (ms: number | null) => (ms === null ? '' : new Date(ms).toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric' }))
</script>

<template>
  <div class="min-h-0 flex-1 overflow-auto p-3" :data-testid="tid('mongo-schema')">
    <p v-if="s.error" role="alert" class="mb-3 rounded-md border border-border bg-surface px-3 py-2 text-xs text-danger" :data-testid="tid('mongo-schema-error')">{{ s.error }}</p>

    <section aria-labelledby="mg-schema-h" class="mb-6">
      <div class="mb-2 flex items-center gap-3">
        <h2 id="mg-schema-h" class="text-sm font-semibold">Fields</h2>
        <span v-if="s.schema" class="text-xs text-muted">sampled {{ s.schema.sampled }} documents</span>
        <span class="ml-auto" />
        <label class="flex items-center gap-1.5 text-xs text-muted">Sample
          <select v-model.number="s.sampleSize" class="h-7 rounded-md border border-border-input bg-surface px-1.5 text-xs text-text" aria-label="Sample size"><option v-for="n in [50, 200, 1000]" :key="n" :value="n">{{ n }}</option></select>
        </label>
        <Button type="button" size="sm" variant="secondary" :disabled="s.loading" @click="store.loadSchema(tab)">Sample again</Button>
      </div>
      <p v-if="s.loading && !s.schema" class="text-sm text-muted" aria-busy="true">Sampling…</p>
      <table v-else-if="s.schema" class="w-full border-collapse text-xs" aria-label="Fields by type">
        <thead><tr class="text-left text-muted"><th scope="col" class="h-7 border-b border-border px-2 font-medium">Field</th><th scope="col" class="border-b border-border px-2 font-medium">Types</th><th scope="col" class="border-b border-border px-2 font-medium">Present</th><th scope="col" class="border-b border-border px-2 font-medium">Example</th></tr></thead>
        <tbody>
          <tr v-for="f in s.schema.fields" :key="f.path" :data-testid="tid(`mongo-field-${f.path}`)">
            <th scope="row" class="whitespace-nowrap border-b border-border px-2 py-1 text-left font-mono font-normal">{{ f.path }}</th>
            <td class="border-b border-border px-2 py-1">{{ f.types.map(t => f.types.length > 1 ? `${t.name} ${t.count}` : t.name).join(', ') }}</td>
            <td class="border-b border-border px-2 py-1">{{ pct(f.present) }}%</td>
            <td class="max-w-[280px] truncate border-b border-border px-2 py-1 font-mono text-muted">{{ f.example ?? '' }}</td>
          </tr>
        </tbody>
      </table>
      <p v-if="s.schema?.truncated" class="mt-1 text-xs text-muted">More fields than are listed here.</p>
    </section>

    <section aria-labelledby="mg-ix-h" class="mb-6">
      <h2 id="mg-ix-h" class="mb-2 text-sm font-semibold">Indexes</h2>
      <table v-if="s.indexes" class="mb-3 w-full border-collapse text-xs" aria-label="Indexes">
        <thead><tr class="text-left text-muted"><th scope="col" class="h-7 border-b border-border px-2 font-medium">Name</th><th scope="col" class="border-b border-border px-2 font-medium">Keys</th><th scope="col" class="border-b border-border px-2 font-medium">Options</th><th scope="col" class="border-b border-border px-2 font-medium">Used</th><th scope="col" class="w-px border-b border-border px-2"><span class="sr-only">Actions</span></th></tr></thead>
        <tbody>
          <tr v-for="i in s.indexes" :key="i.name" :data-testid="tid(`mongo-index-${i.name}`)">
            <th scope="row" class="whitespace-nowrap border-b border-border px-2 py-1 text-left font-mono font-normal">{{ i.name }}</th>
            <td class="border-b border-border px-2 py-1 font-mono">{{ i.keys }}</td>
            <td class="border-b border-border px-2 py-1 text-muted">{{ [i.unique && 'unique', i.sparse && 'sparse', i.ttl_seconds !== null && `ttl ${i.ttl_seconds}s`, i.partial && 'partial'].filter(Boolean).join(', ') }}</td>
            <td class="border-b border-border px-2 py-1" :title="i.since_ms ? `since ${since(i.since_ms)}` : ''">{{ num(i.uses ?? null) }}<span v-if="i.since_ms" class="text-muted"> since {{ since(i.since_ms) }}</span></td>
            <td class="border-b border-border px-1"><Button v-if="i.name !== '_id_'" type="button" size="icon-sm" variant="ghost" class="text-danger" :aria-label="`Drop index ${i.name}`" @click="store.dropIndex(tab, i.name)"><Trash2 class="size-3.5" aria-hidden="true" /></Button></td>
          </tr>
        </tbody>
      </table>
      <form class="flex flex-wrap items-end gap-2" aria-label="New index" @submit.prevent="store.createIndex(tab)">
        <label class="flex flex-col gap-0.5 text-2xs text-muted">Keys<Input v-model="s.indexForm.keys" class="h-7 w-56 font-mono text-xs text-text" placeholder="{ &quot;status&quot;: 1 }" autocomplete="off" spellcheck="false" aria-label="Index keys" /></label>
        <label class="flex flex-col gap-0.5 text-2xs text-muted">Name (optional)<Input v-model="s.indexForm.name" class="h-7 w-40 text-xs text-text" autocomplete="off" spellcheck="false" aria-label="Index name" /></label>
        <label class="flex items-center gap-1.5 text-xs"><input v-model="s.indexForm.unique" type="checkbox" class="size-3.5 accent-[var(--color-accent)]">Unique</label>
        <label class="flex items-center gap-1.5 text-xs"><input v-model="s.indexForm.sparse" type="checkbox" class="size-3.5 accent-[var(--color-accent)]">Sparse</label>
        <label class="flex flex-col gap-0.5 text-2xs text-muted">TTL seconds<Input v-model="s.indexForm.ttl" class="h-7 w-24 text-xs text-text" inputmode="numeric" autocomplete="off" aria-label="TTL seconds" /></label>
        <label class="flex flex-col gap-0.5 text-2xs text-muted">Partial filter<Input v-model="s.indexForm.partial" class="h-7 w-48 font-mono text-xs text-text" autocomplete="off" spellcheck="false" aria-label="Partial filter" /></label>
        <Button type="submit" size="sm" variant="secondary" :disabled="!s.indexForm.keys.trim()" :data-testid="tid('mongo-create-index')">Create index…</Button>
      </form>
    </section>

    <section aria-labelledby="mg-val-h">
      <h2 id="mg-val-h" class="mb-2 text-sm font-semibold">Validation</h2>
      <p class="mb-2 text-xs text-muted">Rules are a query that every new or changed document must match, like <code class="font-mono">{ "qty": { "$gt": 0 } }</code>. Empty means no rules.</p>
      <Textarea v-model="s.rules" class="min-h-24 font-mono text-xs" spellcheck="false" aria-label="Validation rules" :data-testid="tid('mongo-rules')" />
      <div class="mt-2 flex flex-wrap items-center gap-2">
        <label class="flex items-center gap-1.5 text-xs text-muted">Level
          <select v-model="s.level" class="h-7 rounded-md border border-border-input bg-surface px-1.5 text-xs text-text" aria-label="Validation level"><option>strict</option><option>moderate</option><option>off</option></select>
        </label>
        <label class="flex items-center gap-1.5 text-xs text-muted">Action
          <select v-model="s.action" class="h-7 rounded-md border border-border-input bg-surface px-1.5 text-xs text-text" aria-label="Validation action"><option>error</option><option>warn</option></select>
        </label>
        <Button type="button" size="sm" variant="secondary" :data-testid="tid('mongo-check-rules')" @click="store.checkRules(tab)">Check existing documents</Button>
        <Button type="button" size="sm" :data-testid="tid('mongo-save-rules')" @click="store.saveRules(tab)">Save rules…</Button>
      </div>
      <p v-if="s.check" class="mt-2 text-xs" :data-testid="tid('mongo-check-result')" role="status">{{ s.check.failing === 0 ? 'Every existing document matches these rules.' : `${num(s.check.failing)} existing ${s.check.failing === 1 ? 'document breaks' : 'documents break'} these rules. They stay as they are; only new and changed documents are checked.` }}</p>
      <ol v-if="s.check && s.check.sample.length" class="m-0 mt-2 flex list-none flex-col gap-2 p-0" aria-label="Documents that break the rules">
        <li v-for="(d, i) in s.check.sample" :key="i" class="rounded-md border border-border bg-surface"><pre class="m-0 max-h-32 overflow-auto whitespace-pre-wrap break-all p-2 font-mono text-xs">{{ d.relaxed }}</pre></li>
      </ol>
    </section>
  </div>
</template>
