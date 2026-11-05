<script setup lang="ts">
// S21: a breadcrumb you can click to jump up, or click the empty end to type a path.
import { Input } from '@/components/ui/input'

const props = defineProps<{ path: string, remote: boolean }>()
const emit = defineEmits<{ go: [path: string] }>()
const editing = ref(false)
const draft = ref('')

const sepChar = computed(() => (props.remote || props.path.startsWith('/') ? '/' : '\\'))
// [{ label, path }] from the root down.
const crumbs = computed(() => {
  const s = sepChar.value
  const parts = props.path.split(s).filter(Boolean)
  const lead = props.path.startsWith(s) ? s : ''
  const out = [{ label: s === '/' ? '/' : parts[0] ?? '', path: s === '/' ? '/' : parts[0] ?? '' }]
  let acc = lead
  const rest = s === '/' ? parts : parts.slice(1)
  for (const p of rest) {
    acc = acc === '/' ? `/${p}` : `${acc}${acc ? s : ''}${p}`
    out.push({ label: p, path: acc })
  }
  return out
})

function edit() {
  draft.value = props.path
  editing.value = true
}
function commit() {
  editing.value = false
  if (draft.value.trim() && draft.value !== props.path) emit('go', draft.value.trim())
}
</script>

<template>
  <nav aria-label="Path" class="flex min-w-0 flex-1 items-center overflow-hidden whitespace-nowrap font-mono text-xs text-muted">
    <Input v-if="editing" v-model="draft" aria-label="Path" class="h-7 font-mono text-xs" autofocus @keydown.enter.prevent="commit" @keydown.esc.prevent="editing = false" @blur="commit" />
    <template v-else>
      <template v-for="(c, i) in crumbs" :key="c.path">
        <button type="button" :class="['rounded px-1 hover:bg-surface-hover hover:text-text', i === crumbs.length - 1 && 'text-text']" @click="emit('go', c.path)">{{ c.label }}</button>
        <span v-if="i < crumbs.length - 1 && !(i === 0 && c.label === sepChar)" class="opacity-50">{{ sepChar }}</span>
      </template>
      <button type="button" class="h-6 flex-1 cursor-text" aria-label="Type a path" @click="edit" />
    </template>
  </nav>
</template>
