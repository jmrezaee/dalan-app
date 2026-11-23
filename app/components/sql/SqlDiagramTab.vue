<script setup lang="ts">
// S40 "Diagram": tables and their foreign keys, a number of steps out from the table picked. Boxes are placed by
// `diagramLayout` (columns by distance); export writes an SVG or PNG through Rust's save dialog (the window never
// touches the file system).
import { Download } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { BOX_W, HEAD, ROW, layout, toSvg } from '~/utils/diagramLayout'
import type { DiagramTab } from '~/stores/sql'

const props = defineProps<{ tabKey: string, diagram: DiagramTab }>()
const store = useSqlStore(props.tabKey)
const d = computed(() => props.diagram)
const lay = computed(() => (d.value.data ? layout(d.value.data, d.value.start, d.value.keysOnly) : null))
const startText = ref(d.value.start.join(', '))
const message = ref('')

const tables = computed(() => store.schemas.flatMap(s => (s.objects ?? []).filter(o => o.kind !== 'schema').map(o => `${s.name}.${o.name}`)))
async function draw() {
  d.value.start = startText.value.split(',').map(x => x.trim()).filter(Boolean)
  d.value.steps = Math.max(0, Math.min(5, Math.floor(Number(d.value.steps) || 0)))
  await store.loadDiagram(d.value)
}

const theme = () => {
  const css = getComputedStyle(document.documentElement)
  const v = (n: string) => css.getPropertyValue(n).trim()
  return { bg: v('--bg'), surface: v('--surface'), border: v('--border'), text: v('--text'), muted: v('--text-muted'), accent: v('--accent') }
}
const fileName = computed(() => `${(d.value.start[0] ?? 'diagram').replace(/[^\w.-]+/g, '_')}-diagram`)
async function exportSvg() {
  if (!lay.value) return
  const svg = toSvg(lay.value, theme())
  const saved = await store.saveExport(`${fileName.value}.svg`, new TextEncoder().encode(svg)).catch((e: { message?: string }) => { message.value = e.message ?? 'Couldn\'t save it.'; return null })
  if (saved) message.value = `Saved ${saved}`
}
async function exportPng() {
  if (!lay.value) return
  const svg = toSvg(lay.value, theme())
  const img = new Image()
  img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`
  await img.decode().catch(() => {})
  const scale = 2
  const canvas = document.createElement('canvas')
  canvas.width = lay.value.width * scale
  canvas.height = lay.value.height * scale
  canvas.getContext('2d')!.drawImage(img, 0, 0, canvas.width, canvas.height)
  const blob: Blob | null = await new Promise(r => canvas.toBlob(r, 'image/png'))
  if (!blob) { message.value = 'Couldn\'t draw the picture.'; return }
  const saved = await store.saveExport(`${fileName.value}.png`, new Uint8Array(await blob.arrayBuffer())).catch((e: { message?: string }) => { message.value = e.message ?? 'Couldn\'t save it.'; return null })
  if (saved) message.value = `Saved ${saved}`
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col" :data-testid="tid('sql-diagram')">
    <form class="flex min-h-11 shrink-0 flex-wrap items-center gap-2 border-b border-border px-3 py-1.5" @submit.prevent="draw">
      <label :for="`${tabKey}-${d.id}-from`" class="text-xs text-muted">From</label>
      <Input :id="`${tabKey}-${d.id}-from`" v-model="startText" list="diagram-tables" class="h-7 w-64 font-mono text-xs" placeholder="public.orders" autocomplete="off" spellcheck="false" />
      <datalist id="diagram-tables"><option v-for="t in tables" :key="t" :value="t" /></datalist>
      <label :for="`${tabKey}-${d.id}-steps`" class="text-xs text-muted">Steps</label>
      <Input :id="`${tabKey}-${d.id}-steps`" v-model.number="d.steps" type="number" min="0" max="5" class="h-7 w-16 text-xs" />
      <label class="flex items-center gap-1.5 text-xs text-muted"><input v-model="d.keysOnly" type="checkbox">Keys only</label>
      <Button type="submit" size="sm" :loading="d.loading">Draw</Button>
      <span v-if="d.data" class="text-xs text-muted" data-part="count">{{ d.data.tables.length }} of {{ d.data.total_tables }} tables</span>
      <span class="ml-auto flex items-center gap-1">
        <Button variant="secondary" size="sm" :disabled="!lay" @click="exportSvg"><Download />SVG</Button>
        <Button variant="secondary" size="sm" :disabled="!lay" @click="exportPng"><Download />PNG</Button>
      </span>
    </form>
    <p v-if="message" role="status" class="shrink-0 border-b border-border px-3 py-1 text-xs text-muted">{{ message }}</p>
    <p v-if="d.error" role="alert" class="shrink-0 border-b border-border px-3 py-1.5 text-xs text-danger">{{ d.error }}</p>
    <div v-if="lay" class="min-h-0 flex-1 overflow-auto bg-bg">
      <svg :width="lay.width" :height="lay.height" role="img" :aria-label="`Diagram of ${lay.boxes.length} tables`" class="text-text">
        <path v-for="k in lay.links" :key="k.name" :d="k.d" fill="none" stroke="var(--accent)" stroke-width="1.5"><title>{{ k.label }}</title></path>
        <g v-for="b in lay.boxes" :key="b.id" :data-testid="tid(`diagram-box-${b.name}`)">
          <rect :x="b.x" :y="b.y" :width="b.w" :height="b.h" rx="8" fill="var(--surface)" stroke="var(--border)" />
          <text :x="b.x + 12" :y="b.y + 20" font-weight="600" fill="var(--text)" font-size="12">{{ b.schema === 'public' ? b.name : `${b.schema}.${b.name}` }}</text>
          <template v-for="(r, i) in b.rows" :key="r.name">
            <text :x="b.x + 12" :y="b.y + HEAD + i * ROW + 14" font-family="var(--font-code)" font-size="11" :fill="r.pk || r.fk ? 'var(--accent)' : 'var(--text)'">{{ r.name }}</text>
            <text :x="b.x + BOX_W - 12" :y="b.y + HEAD + i * ROW + 14" text-anchor="end" font-family="var(--font-code)" font-size="11" fill="var(--text-muted)">{{ r.type_name }}</text>
          </template>
        </g>
      </svg>
    </div>
    <p v-else-if="!d.loading && !d.error" class="p-4 text-sm text-muted">Pick a table and press Draw: its foreign keys are followed the number of steps you choose.</p>
  </div>
</template>
