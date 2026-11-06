<script setup lang="ts">
// S19: several sessions in one tab. Split shows them all in a grid; Focus shows one big with the rest listed.
// Broadcast types into every live pane at once. Every pane's terminal stays mounted in both modes.
import { Plus, Radio, Save } from '@lucide/vue'
import type { Session } from '~/stores/sessions'
import { MAX_PANES } from '~/stores/sessions'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'
const { label: shortcutLabel } = useShortcutLabel()

const props = defineProps<{ ws: Session, active: boolean }>()
const emit = defineEmits<{ editHost: [hostId: string], newHost: [] }>()
const sessions = useSessionsStore()
const hosts = useHostsStore()
const templates = useWorkspacesStore()

const cols = computed(() => {
  const n = props.ws.panes.length
  return n <= 1 ? 1 : n <= 4 ? 2 : n <= 9 ? 3 : 4
})
const rows = computed(() => Math.max(1, Math.ceil(props.ws.panes.length / cols.value)))

// ---- Resizable splits. The panes stay in one CSS grid (so no terminal is ever remounted, hard rule 4);
// the track sizes are fractions we change by dragging the thin handles between panes.
const colFr = ref<number[]>([])
const rowFr = ref<number[]>([])
watch(cols, c => { colFr.value = Array.from({ length: c }, () => 1) }, { immediate: true })
watch(rows, r => { rowFr.value = Array.from({ length: r }, () => 1) }, { immediate: true })
const gridStyle = computed(() => (props.ws.mode === 'split'
  ? { gridTemplateColumns: colFr.value.map(f => `minmax(0, ${f}fr)`).join(' '), gridTemplateRows: rowFr.value.map(f => `minmax(0, ${f}fr)`).join(' ') }
  : { gridTemplateColumns: 'minmax(0, 1fr)', gridAutoRows: 'minmax(0, 1fr)' }))
const grid = ref<HTMLElement | null>(null)
const MIN_FR = 0.25
/** Where handle `i` (between track i-1 and i) sits, as a percentage of the grid. */
const edgePct = (fr: number[], i: number) => (fr.slice(0, i).reduce((a, b) => a + b, 0) / fr.reduce((a, b) => a + b, 0)) * 100
function dragHandle(e: PointerEvent, axis: 'col' | 'row', i: number) {
  const el = grid.value
  if (!el) return
  const fr = axis === 'col' ? colFr : rowFr
  const size = axis === 'col' ? el.clientWidth : el.clientHeight
  const total = fr.value.reduce((a, b) => a + b, 0)
  const start = axis === 'col' ? e.clientX : e.clientY
  const before = fr.value[i - 1]!
  const after = fr.value[i]!
  const target = e.currentTarget as HTMLElement
  target.setPointerCapture(e.pointerId)
  const move = (m: PointerEvent) => {
    const d = (((axis === 'col' ? m.clientX : m.clientY) - start) / size) * total
    const next = Math.min(Math.max(before + d, MIN_FR), before + after - MIN_FR)
    const copy = [...fr.value]
    copy[i - 1] = next
    copy[i] = before + after - next
    fr.value = copy
  }
  const up = () => { target.removeEventListener('pointermove', move); target.removeEventListener('pointerup', up) }
  target.addEventListener('pointermove', move)
  target.addEventListener('pointerup', up)
}

const dot = paneDot
const dotLabel = (p: Session) => paneDotLabel(p)
const hint = (p: Session) => (p.prompt ? 'needs an answer' : p.status === 'ready' ? 'connected' : p.status === 'disconnected' ? 'disconnected' : p.status === 'error' ? 'failed' : 'connecting')

const live = computed(() => props.ws.panes.filter(p => p.status === 'ready'))
const skipped = computed(() => props.ws.panes.filter(p => p.status !== 'ready'))
const broadcastHint = computed(() => {
  if (!props.ws.broadcast) return 'Typing goes to the focused pane only.'
  const skip = skipped.value.length ? ` ${skipped.value.map(p => p.label).join(', ')} ${skipped.value.length === 1 ? 'is' : 'are'} not connected and skipped.` : ''
  return `Typing goes to ${live.value.length} live ${live.value.length === 1 ? 'pane' : 'panes'}.${skip}`
})

// ---- save as template (F09)
const saveOpen = ref(false)
const saveName = ref('')
const saveError = ref('')
function askSave() {
  saveName.value = props.ws.templateId ? props.ws.label : ''
  saveError.value = ''
  saveOpen.value = true
}
async function doSave() {
  saveError.value = await templates.save(props.ws, saveName.value)
  if (!saveError.value) saveOpen.value = false
}
defineExpose({ askSave })

function maximize(i: number) {
  sessions.focusPane(props.ws, i)
  sessions.setMode(props.ws, 'focus')
}
</script>

<template>
  <div class="flex size-full flex-col bg-terminal-bg">
    <div class="flex h-10 shrink-0 flex-wrap items-center gap-2 border-b border-border bg-terminal-bg px-3">
      <Segmented :model-value="ws.mode" label="Layout" class="w-44" :options="[{ value: 'split', label: 'Split' }, { value: 'focus', label: 'Focus' }]" @update:model-value="(v: string) => sessions.setMode(ws, v as 'split' | 'focus')" />
      <Button size="sm" :variant="ws.broadcast ? 'outline' : 'secondary'" :aria-pressed="ws.broadcast" :class="ws.broadcast && 'border-warning/50 bg-warning/15 text-warning'" @click="sessions.toggleBroadcast(ws)"><Radio />Broadcast<Kbd>{{ shortcutLabel('broadcast') }}</Kbd></Button>
      <StatusPill v-if="ws.broadcast" tone="warning"><Radio class="size-3" aria-hidden="true" />Broadcasting to {{ live.length }}</StatusPill>
      <span class="truncate text-xs text-muted">{{ broadcastHint }}</span>
      <div class="ml-auto flex items-center gap-2">
        <DropdownMenu>
          <DropdownMenuTrigger as-child><Button size="sm" variant="secondary" :disabled="ws.panes.length >= MAX_PANES"><Plus />Add pane</Button></DropdownMenuTrigger>
          <DropdownMenuContent align="end" class="max-h-72 overflow-auto">
            <DropdownMenuItem v-for="h in hosts.hosts" :key="h.id" @select="sessions.addPane(ws, h)">{{ h.label }}</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <Button size="sm" variant="secondary" @click="askSave"><Save />Save as template<Kbd>{{ shortcutLabel('save-template') }}</Kbd></Button>
      </div>
    </div>

    <div class="flex min-h-0 flex-1">
      <div ref="grid" class="relative grid min-w-0 flex-1 gap-1 bg-border" :style="gridStyle">
        <template v-if="ws.mode === 'split'">
          <div
            v-for="i in cols - 1"
            :key="`c${i}`"
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize columns"
            class="absolute inset-y-0 z-10 w-2 -translate-x-1/2 cursor-col-resize hover:bg-accent/40"
            :style="{ left: `${edgePct(colFr, i)}%` }"
            @pointerdown.prevent="dragHandle($event, 'col', i)"
          />
          <div
            v-for="i in rows - 1"
            :key="`r${i}`"
            role="separator"
            aria-orientation="horizontal"
            aria-label="Resize rows"
            class="absolute inset-x-0 z-10 h-2 -translate-y-1/2 cursor-row-resize hover:bg-accent/40"
            :style="{ top: `${edgePct(rowFr, i)}%` }"
            @pointerdown.prevent="dragHandle($event, 'row', i)"
          />
        </template>
        <section
          v-for="(p, i) in ws.panes"
          v-show="ws.mode === 'split' || i === ws.focus"
          :key="p.key"
          :aria-label="p.label"
          :class="cn('flex min-h-0 min-w-0 flex-col bg-terminal-bg', i === ws.focus && 'ring-1 ring-inset ring-accent', ws.broadcast && p.status === 'ready' && 'outline-dashed outline-2 -outline-offset-2 outline-warning')"
          @focusin="sessions.focusPane(ws, i)"
          @mousedown="sessions.focusPane(ws, i)"
        >
          <PaneHeader
            :title="`${p.label}  ${p.address}`"
            :status="dot(p)"
            :status-label="dotLabel(p)"
            :focused="i === ws.focus"
            :broadcasting="ws.broadcast && p.status === 'ready'"
            class="rounded-none border-x-0 border-b border-t-0"
            @maximize="maximize(i)"
            @close="sessions.closePane(ws, p)"
          />
          <div class="min-h-0 flex-1">
            <SessionPane :session="p" :active="active && i === ws.focus" embedded @edit-host="emit('editHost', $event)" @new-host="emit('newHost')" />
          </div>
        </section>
      </div>

      <aside v-if="ws.mode === 'focus'" aria-label="Other panes" class="flex w-[240px] shrink-0 flex-col gap-1 border-l border-border bg-surface p-2">
        <button
          v-for="(p, i) in ws.panes"
          v-show="i !== ws.focus"
          :key="p.key"
          type="button"
          class="flex items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm hover:bg-surface-hover"
          @click="sessions.focusPane(ws, i)"
        >
          <StatusDot :status="dot(p)" :label="dotLabel(p)" />
          <span class="truncate">{{ p.label }}</span>
          <span class="ml-auto shrink-0 text-xs text-muted">{{ hint(p) }}</span>
        </button>
      </aside>
      <SessionSidePanel v-if="sessions.panelOpen" />
    </div>

    <Dialog v-model:open="saveOpen">
      <DialogContent class="sm:max-w-[380px]">
        <DialogHeader><DialogTitle>Save workspace as a template</DialogTitle></DialogHeader>
        <form class="flex flex-col gap-3" @submit.prevent="doSave">
          <Input v-model="saveName" aria-label="Template name" placeholder="Staging ×4" autofocus autocomplete="off" />
          <p class="text-xs text-muted">Remembers which servers are open and the layout. Reopen it from New Tab or the command palette.</p>
          <p v-if="saveError" role="alert" class="text-sm text-danger">{{ saveError }}</p>
          <div class="flex justify-end gap-2"><Button type="button" variant="secondary" @click="saveOpen = false">Cancel</Button><Button type="submit">Save</Button></div>
        </form>
      </DialogContent>
    </Dialog>
  </div>
</template>
