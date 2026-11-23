<script setup lang="ts">
// S39 "Explain": the plan as a tree with each step's own time and share, the planner's estimate against what it
// found, and a plain-language hint with actions that open SQL in the editor (nothing runs from here).
import { cn } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import type { PlanNode_Serialize as PlanNode } from '~/types/bindings'
import type { PlanState } from '~/stores/sql'

const props = defineProps<{ plan: PlanState }>()
const emit = defineEmits<{ open: [sql: string], rerun: [analyze: boolean] }>()
const mode = ref<'tree' | 'text'>('tree')
const p = computed(() => props.plan.plan)
const fmt = (n: number | null | undefined) => Math.round(n ?? 0).toLocaleString('en-US')
const ms = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(2)} s` : n >= 10 ? `${n.toFixed(0)} ms` : `${n.toFixed(1)} ms`)

interface Row { node: PlanNode, depth: number }
function flatten(n: PlanNode, depth = 0, out: Row[] = []): Row[] {
  out.push({ node: n, depth })
  for (const c of n.children) flatten(c, depth + 1, out)
  return out
}
const rows = computed(() => (p.value ? flatten(p.value.root) : []))
const text = computed(() => rows.value.map(({ node: n, depth }) => `${'  '.repeat(depth)}${depth ? '-> ' : ''}${n.name}  (rows=${fmt(n.est_rows)}${n.actual_rows != null ? `, actual=${fmt(n.actual_rows)}` : ''}${n.self_ms != null ? `, time=${ms(n.self_ms)}` : ''})${n.detail ? `\n${'  '.repeat(depth + 1)}${n.detail}` : ''}`).join('\n'))
// An estimate that is far off (ten times either way) is marked.
const off = (n: PlanNode) => {
  const a = n.actual_rows, e = n.est_rows ?? 0
  return a !== null && a !== undefined && (a > e * 10 || a * 10 < e) && Math.abs(a - e) > 100
}
</script>

<template>
  <div class="flex min-w-0 flex-1 flex-col overflow-auto p-3 text-sm" :data-testid="tid('sql-plan')">
    <p v-if="plan.state === 'loading'" class="text-muted" aria-busy="true">Asking the server for a plan…</p>
    <div v-else-if="plan.state === 'failed'" role="alert" class="rounded-md border border-danger/40 px-3 py-2 text-danger">{{ plan.error }}</div>
    <template v-else-if="p">
      <div class="mb-2 flex flex-wrap items-center gap-3">
        <b class="font-medium">{{ p.analyzed ? `${ms(p.execution_ms ?? 0)} total` : 'Plan only' }}</b>
        <span class="text-xs text-muted">{{ p.analyzed ? `${fmt(p.buffers_read ?? 0)} buffers read · ran with explain (analyze, buffers) inside a rolled-back transaction` : 'The statement was not run.' }}</span>
        <span class="ml-auto flex gap-1">
          <button v-for="m in (['tree', 'text'] as const)" :key="m" type="button" :aria-pressed="mode === m" :class="cn('h-6 cursor-default rounded-md px-2 text-xs capitalize text-muted', mode === m && 'bg-surface-hover text-text')" @click="mode = m">{{ m }}</button>
          <Button v-if="!p.analyzed" variant="secondary" size="xs" @click="emit('rerun', true)">Run with timings</Button>
        </span>
      </div>
      <ul v-if="mode === 'tree'" class="m-0 flex list-none flex-col p-0" aria-label="Plan">
        <li v-for="(r, i) in rows" :key="i" :class="cn('flex items-start gap-3 border-t border-surface-raised py-1.5', (r.node.pct ?? 0) >= 50 && 'bg-warning/10')" :style="{ paddingLeft: `${r.depth * 20}px` }" :data-testid="tid('plan-node')">
          <span class="min-w-0 flex-1"><span class="font-medium">{{ r.node.name }}</span><br><span v-if="r.node.detail" class="break-all font-mono text-xs text-muted">{{ r.node.detail }}</span></span>
          <span v-if="r.node.self_ms != null" class="w-28 shrink-0 text-right font-mono text-xs">{{ ms(r.node.self_ms) }} · {{ Math.round(r.node.pct ?? 0) }}%</span>
          <span class="w-44 shrink-0 text-right font-mono text-xs text-muted">est {{ fmt(r.node.est_rows) }}<template v-if="r.node.actual_rows != null"> · got <span :class="off(r.node) ? 'text-warning' : 'text-text'">{{ fmt(r.node.actual_rows) }}</span></template></span>
        </li>
      </ul>
      <pre v-else class="overflow-auto whitespace-pre rounded-md border border-border bg-surface p-3 font-mono text-xs">{{ text }}</pre>
      <div v-if="p.hint" class="mt-3 rounded-md border border-border bg-surface p-3" :data-testid="tid('plan-hint')">
        <p>{{ p.hint.text }}</p>
        <div class="mt-2 flex gap-2"><Button v-for="a in p.hint.actions" :key="a.label" variant="secondary" size="sm" @click="emit('open', a.sql)">{{ a.label }}</Button></div>
      </div>
    </template>
  </div>
</template>
