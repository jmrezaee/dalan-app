<script setup lang="ts">
// The table every collection viewer draws (hash, list, set, sorted set, stream): mono cells, virtualised, read-only.
// A cell is a value's bytes (text as it is, binary as hex with a tag) or a plain label (a position, a score, an id).
import type { RedisBytes } from '~/types/bindings'
import { bytesOf, bytesText } from '~/utils/redisBytes'
import { cn } from '@/lib/utils'

export type Cell = RedisBytes | string
const props = defineProps<{ label: string, columns: { label: string, width?: string }[], rows: Cell[][] }>()

const ROW = 30
const { scroller, onScroll, from, to, padTop, padBottom, reveal } = useVirtualRows(computed(() => props.rows.length), ROW)
const shown = computed(() => props.rows.slice(from.value, to.value))
const focused = ref(0)
watch(() => props.rows.length, (n) => { if (focused.value >= n) focused.value = Math.max(0, n - 1) })
function onKey(e: KeyboardEvent) {
  const d = e.key === 'ArrowDown' ? 1 : e.key === 'ArrowUp' ? -1 : e.key === 'PageDown' ? 20 : e.key === 'PageUp' ? -20 : 0
  if (!d) return
  e.preventDefault()
  focused.value = Math.min(props.rows.length - 1, Math.max(0, focused.value + d))
  reveal(focused.value)
}
const isBinary = (c: Cell) => typeof c !== 'string' && c.kind === 'binary'
const show = (c: Cell) => (typeof c === 'string' ? c : c.kind === 'text' ? c.text : Array.from(bytesOf(c).subarray(0, 64), n => n.toString(16).padStart(2, '0')).join(' '))
const title = (c: Cell) => (typeof c === 'string' ? c : bytesText(c).slice(0, 2000))
const grid = computed(() => ({ gridTemplateColumns: props.columns.map(c => c.width ?? 'minmax(0, 1fr)').join(' ') }))
</script>

<template>
  <div ref="scroller" role="table" :aria-label="label" :aria-rowcount="rows.length + 1" tabindex="0" class="min-h-0 flex-1 overflow-auto outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" @scroll="onScroll" @keydown="onKey">
    <div role="row" class="sticky top-0 z-10 grid h-7 items-center border-b border-border bg-bg text-xs font-medium text-muted" :style="grid">
      <span v-for="c in columns" :key="c.label" role="columnheader" class="px-2.5">{{ c.label }}</span>
    </div>
    <div :style="{ height: `${padTop}px` }" aria-hidden="true" />
    <div
      v-for="(r, i) in shown"
      :key="from + i"
      role="row"
      :aria-rowindex="from + i + 2"
      :class="cn('grid h-[30px] items-center border-b border-surface-raised', from + i === focused && 'bg-surface')"
      :style="grid"
      @click="focused = from + i"
    >
      <span v-for="(c, j) in r" :key="j" role="cell" :class="cn('truncate px-2.5 font-mono text-xs', (typeof c === 'string' || isBinary(c)) && 'text-muted')" :title="title(c)">
        {{ show(c) }}<span v-if="isBinary(c)" class="ml-2 rounded border border-border px-1 font-sans text-2xs">binary</span>
      </span>
    </div>
    <div :style="{ height: `${padBottom}px` }" aria-hidden="true" />
  </div>
</template>
