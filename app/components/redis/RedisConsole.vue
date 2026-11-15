<script setup lang="ts">
// S30: the Console. Our own input and output (not a terminal): a line goes to Rust, which parses it, asks the
// write gate and prints the reply. Nothing typed here can skip the gate; this view only shows what it answers.
import { cn } from '@/lib/utils'
import { count } from '~/utils/redisBytes'
import type { ConsoleEntry } from '~/stores/redis'

const props = defineProps<{ tabKey: string, dbName: string, production: boolean }>()
const store = useRedisStore(props.tabKey)


const line = ref('')
const input = ref<HTMLInputElement | null>(null)
const output = ref<HTMLElement | null>(null)
const isMac = typeof navigator !== 'undefined' && /Mac/.test(navigator.platform)
const prompt = computed(() => `${props.dbName}:${store.selected}>`)
const SHOWN = 1000
onMounted(() => {
  void store.loadConsole()
  input.value?.focus()
})

// New output scrolls into view.
watch(() => [store.consoleEntries.length, store.consoleEntries.at(-1)?.state], () => nextTick(() => { if (output.value) output.value.scrollTop = output.value.scrollHeight }))

// ---- Completion: while the first word is typed, the commands that start with it; after it, that command's syntax.
const popupOff = ref(false)
const word = computed(() => line.value.trimStart().split(/\s+/)[0]?.toUpperCase() ?? '')
const typingName = computed(() => !!word.value && !/\s/.test(line.value.trimStart()))
const matches = computed(() => (typingName.value && !popupOff.value ? store.commandList.filter(c => c.name.startsWith(word.value)).slice(0, 6) : []))
const known = computed(() => (typingName.value ? null : store.commandList.find(c => c.name === word.value) ?? null))
watch(line, () => { popupOff.value = false })
function complete() {
  const top = matches.value[0]
  if (top) line.value = `${top.name} `
}

// ---- History: ↑ goes back, ↓ comes forward; what was being typed is kept at the bottom.
let at = -1
let draft = ''
function walk(d: number) {
  const h = store.consoleHistory
  const next = at + d
  if (next < -1 || next >= h.length) return
  if (at === -1) draft = line.value
  at = next
  line.value = at === -1 ? draft : h[at]!
}
function run() {
  const text = line.value
  line.value = ''
  at = -1
  void store.consoleRun(text)
}
function onKey(e: KeyboardEvent) {
  if (e.key === 'Tab' && matches.value.length) {
    e.preventDefault()
    complete()
  }
  else if (e.key === 'ArrowUp') {
    e.preventDefault()
    walk(1)
  }
  else if (e.key === 'ArrowDown') {
    e.preventDefault()
    walk(-1)
  }
  else if (e.key === 'Escape' && matches.value.length) {
    e.preventDefault()
    popupOff.value = true
  }
  else if (e.key.toLowerCase() === 'l' && (isMac ? e.metaKey : e.ctrlKey)) {
    e.preventDefault()
    store.consoleClear()
  }
}
function use(text: string) {
  line.value = text
  input.value?.focus()
}

// ---- Printing: list numbers and (nil) muted, (integer) in yellow, (error) in danger. Rust already made the text.
function pieces(text: string): { t: string, c: string }[] {
  const m = /^((?:\s*\d+[)#~] )*)(\((?:integer|double|true|false|big number)\)|\(nil\)|\(empty [a-z]+\)|\(error\).*)?(.*)$/s.exec(text)
  if (!m) return [{ t: text, c: '' }]
  const tag = m[2] ?? ''
  return [
    { t: m[1] ?? '', c: 'text-muted' },
    { t: tag, c: tag.startsWith('(error)') ? 'text-danger' : tag === '(nil)' || tag.startsWith('(empty') ? 'text-muted' : 'text-ansi-yellow' },
    { t: m[3] ?? '', c: '' },
  ].filter(p => p.t)
}
const shown = (e: ConsoleEntry) => (e.showAll ? e.lines : e.lines.slice(0, SHOWN))
const heavyWhy = (e: ConsoleEntry) => (/^keys\b/i.test(e.line) ? 'KEYS walks every key at once and can stall a busy server.' : 'This reads a collection of more than 10,000 items in one go, which can stall a busy server.')
</script>

<template>
  <div class="flex min-h-0 flex-1" :data-testid="tid('redis-console')">
    <section aria-label="Console" class="flex min-w-0 flex-1 flex-col bg-terminal-bg text-terminal-fg" @click="($event.target as HTMLElement).closest('button, a, [data-output]') || input?.focus()">
      <div ref="output" role="log" aria-live="polite" aria-label="Console output" class="min-h-0 flex-1 select-text overflow-auto px-4 py-3 font-mono text-xs leading-5" data-output>
        <p v-if="!store.consoleEntries.length" class="font-sans text-muted">Type a command and press Enter. Reads run at once; anything that changes data follows this database's write mode.</p>
        <div v-for="e in store.consoleEntries" :key="e.id" class="mb-2.5" :data-testid="tid('console-entry')" :data-state="e.state">
          <div class="whitespace-pre-wrap break-all"><span class="text-accent">{{ dbName }}:{{ e.prompt }}</span> {{ e.line }}</div>

          <div v-if="e.state === 'running'" class="text-muted">…</div>
          <div v-else-if="e.state === 'refused'" class="whitespace-pre-wrap text-danger">{{ e.lines[0]?.startsWith('Not sent') ? e.lines[0] : `Not sent. ${e.lines[0] ?? ''}` }}</div>
          <div v-else-if="e.state === 'cancelled'" class="text-muted">{{ e.lines[0] }}</div>
          <div v-else-if="e.state === 'pending' && e.ask?.heavy" class="my-1 rounded-md bg-warning/10 px-2.5 py-2 font-sans text-warning">
            Not sent. {{ heavyWhy(e) }}
            <template v-if="e.ask.safer"><button type="button" class="ml-1 text-accent underline" @click="use(e.ask.safer); store.consoleCancel(e.id)">Use {{ e.ask.safer.split(' ')[0] }} instead</button> ·</template>
            <button type="button" class="ml-1 underline" @click="store.consoleSend(e.id)">Run anyway…</button>
          </div>
          <div v-else-if="e.state === 'pending'" class="my-1 flex flex-wrap items-center gap-2 rounded-md bg-accent/[0.08] px-2.5 py-2 font-sans">
            <span>This changes data<template v-if="production"> on <b class="font-semibold text-warning">production</b></template>.</span>
            <Button size="sm" @click="store.consoleSend(e.id)">Send</Button>
            <Button variant="ghost" size="sm" @click="store.consoleCancel(e.id)">Cancel</Button>
          </div>
          <template v-else>
            <div v-for="(l, i) in shown(e)" :key="i" class="min-h-5 whitespace-pre-wrap break-all"><span v-for="(p, j) in pieces(l)" :key="j" :class="p.c">{{ p.t }}</span></div>
            <div class="text-muted">
              <button v-if="e.lines.length > SHOWN && !e.showAll" type="button" class="mr-2 font-sans text-accent underline" @click="e.showAll = true">Show all {{ count(e.lines.length) }} lines</button>
              <span v-if="e.truncated" class="mr-2 font-sans">The reply was longer; the rest was left out.</span>
              <span v-if="e.elapsedMs !== null">· {{ e.elapsedMs < 10 ? e.elapsedMs.toFixed(1) : Math.round(e.elapsedMs) }} ms</span>
            </div>
          </template>
        </div>
      </div>

      <form class="relative flex shrink-0 items-center gap-2 border-t border-border px-4 py-2.5" @submit.prevent="run">
        <div v-if="matches.length" class="absolute bottom-[42px] left-4 z-10 w-[460px] max-w-[calc(100%-2rem)] overflow-hidden rounded-lg border border-border bg-surface-raised shadow-pop" role="listbox" aria-label="Commands" :data-testid="tid('console-complete')">
          <div v-for="(c, i) in matches" :key="c.name" role="option" :aria-selected="i === 0" :class="cn('flex cursor-default gap-2.5 px-2.5 py-1.5', i === 0 && 'bg-accent/15')" @mousedown.prevent="use(`${c.name} `)">
            <span class="font-mono text-xs font-semibold">{{ c.name }}</span><span class="truncate font-mono text-xs text-muted">{{ c.syntax }}</span>
          </div>
          <div class="flex gap-3 border-t border-border px-2.5 py-1.5 text-xs text-muted"><span class="min-w-0 flex-1 truncate">{{ matches[0]?.summary ?? (matches[0]?.write ? 'Changes data.' : '') }}</span><span class="shrink-0">Tab to complete</span></div>
        </div>
        <div v-else-if="known && known.syntax" class="absolute bottom-[42px] left-4 z-10 max-w-[calc(100%-2rem)] truncate rounded-md border border-border bg-surface-raised px-2.5 py-1 font-mono text-xs text-muted shadow-pop" :data-testid="tid('console-syntax')"><b class="font-semibold text-text">{{ known.name }}</b> {{ known.syntax }}</div>

        <label :for="`${tabKey}-console-input`" class="shrink-0 font-mono text-xs text-accent">{{ prompt }}</label>
        <input :id="`${tabKey}-console-input`" ref="input" v-model="line" class="min-w-0 flex-1 bg-transparent font-mono text-xs outline-none" autocomplete="off" autocapitalize="off" spellcheck="false" aria-label="Command" @keydown="onKey">
        <span class="shrink-0 text-xs text-muted">↑ history · {{ isMac ? '⌘L' : 'Ctrl+L' }} clear</span>
      </form>
    </section>

    <aside aria-label="History" class="flex w-[300px] shrink-0 flex-col border-l border-border bg-surface">
      <div class="flex h-9 shrink-0 items-center gap-2 border-b border-border px-3"><span class="font-medium">History</span><span class="text-xs text-muted">this database</span><Button v-if="store.consoleHistory.length" variant="ghost" size="sm" class="ml-auto" @click="store.consoleClearHistory()">Clear</Button></div>
      <ul class="min-h-0 flex-1 overflow-auto py-1.5">
        <li v-if="!store.consoleHistory.length" class="px-3 py-1 text-xs text-muted">Nothing yet. Commands you run show up here.</li>
        <li v-for="(h, i) in store.consoleHistory" :key="i"><button type="button" class="block h-[26px] w-full truncate px-3 text-left font-mono text-xs hover:bg-surface-hover" :title="h" @click="use(h)">{{ h }}</button></li>
      </ul>
      <p class="shrink-0 border-t border-border p-3 text-xs text-muted">History is kept encrypted, like terminal history. Commands that carry a password (AUTH, ACL SETUSER, CONFIG SET requirepass) are never saved.</p>
    </aside>
  </div>
</template>
