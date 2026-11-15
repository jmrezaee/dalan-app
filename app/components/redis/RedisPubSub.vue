<script setup lang="ts">
// S32: listen to channels and patterns, and publish. Listening uses its own connection in Rust, so the Browser
// and the Console keep working. Publishing is a write: it goes through the gate like every other.
import { Search, X } from '@lucide/vue'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'
import { bytesText, count } from '~/utils/redisBytes'
import { KEEP_MESSAGES } from '~/stores/redis'

const props = defineProps<{ tabKey: string, readOnly: boolean, saveLabel: string }>()
const store = useRedisStore(props.tabKey)

const name = ref('')
async function listen() {
  if (await store.listen(name.value)) name.value = ''
}

const filter = ref('')
const rows = computed(() => {
  const q = filter.value.trim().toLowerCase()
  const all = store.messages
  return q ? all.filter(m => bytesText(m.channel).toLowerCase().includes(q) || bytesText(m.payload).toLowerCase().includes(q)) : all
})
const ROW = 30
const { scroller, onScroll, from, to, padTop, padBottom } = useVirtualRows(computed(() => rows.value.length), ROW)
const shown = computed(() => rows.value.slice(from.value, to.value))
const time = (ms: number) => {
  const d = new Date(ms)
  const two = (n: number) => String(n).padStart(2, '0')
  return `${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())}.${String(d.getMilliseconds()).padStart(3, '0')}`
}

const channel = ref('')
const text = ref('')
async function publish() {
  if (!channel.value.trim()) return
  // In Confirm mode this opens the review dialog; the fields are cleared once the message is on its way.
  if (await store.publish(channel.value.trim(), text.value)) text.value = ''
}
watch(() => store.pendingWrite, (now, before) => { if (before?.op?.op === 'publish' && !now) text.value = '' })
</script>

<template>
  <div class="flex min-h-0 flex-1" :data-testid="tid('redis-pubsub')">
    <section aria-label="Listening to" class="flex w-[300px] shrink-0 flex-col border-r border-border">
      <form class="flex gap-1.5 border-b border-border px-3 py-2.5" @submit.prevent="listen">
        <Input v-model="name" aria-label="Channel or pattern" class="h-7 min-w-0 flex-1 font-mono text-xs" placeholder="channel or pat*ern" autocomplete="off" spellcheck="false" />
        <Button size="sm" type="submit" :disabled="!name.trim()">Listen</Button>
      </form>
      <p v-if="store.subError" role="alert" class="px-3 pt-2 text-xs text-danger">{{ store.subError }}</p>
      <ul class="min-h-0 flex-1 overflow-auto py-1.5">
        <li v-if="!store.subs.length" class="px-3 py-1 text-xs text-muted">Not listening to anything yet.</li>
        <li v-for="s in store.subs" :key="`${s.pattern}:${s.name}`" class="flex h-9 items-center gap-2 px-3">
          <span :class="cn('size-2 shrink-0 rounded-full', s.live ? 'bg-success' : 'bg-border-input')" :title="s.live ? 'Listening' : 'Stopped'" aria-hidden="true" />
          <span class="min-w-0 truncate font-mono text-xs" :title="s.name">{{ s.name }}</span>
          <span class="shrink-0 text-2xs text-muted">{{ s.pattern ? 'pattern' : 'channel' }}<span class="sr-only">, {{ s.live ? 'listening' : 'stopped' }}</span></span>
          <span class="ml-auto shrink-0 text-xs tabular-nums text-muted">{{ count(s.count) }}</span>
          <IconButton size="xs" :tip="false" :label="`Stop listening to ${s.name}`" @click="store.unlisten(s)"><X /></IconButton>
        </li>
      </ul>
      <p class="shrink-0 border-t border-border p-3 text-xs text-muted">Listening uses its own connection, so the Browser and Console keep working. It stops when you close the tab or lock Dalan.</p>
    </section>

    <section aria-label="Messages" class="flex min-w-0 flex-1 flex-col">
      <div class="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
        <h2 class="font-medium">Messages</h2>
        <span class="truncate text-xs text-muted" aria-live="off">{{ count(store.received) }} received · keeping the last {{ count(KEEP_MESSAGES) }}<template v-if="store.lost"> · {{ count(store.lost) }} dropped</template><template v-if="store.paused"> · Paused · {{ count(store.heldMessages.length) }} waiting</template></span>
        <label class="relative ml-auto w-[200px] shrink-0"><Search class="pointer-events-none absolute left-2 top-1.5 size-4 text-muted" aria-hidden="true" /><Input v-model="filter" aria-label="Filter messages" class="h-7 pl-8 text-xs" placeholder="Filter messages" autocomplete="off" spellcheck="false" /></label>
        <Button variant="secondary" size="sm" :aria-pressed="store.paused" @click="store.setPaused(!store.paused)">{{ store.paused ? 'Resume' : 'Pause' }}</Button>
        <Button variant="ghost" size="sm" @click="store.clearMessages()">Clear</Button>
      </div>

      <div ref="scroller" role="table" aria-label="Messages" :aria-rowcount="rows.length + 1" tabindex="0" class="min-h-0 flex-1 overflow-auto outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent" @scroll="onScroll">
        <div role="row" class="sticky top-0 z-10 grid h-7 grid-cols-[120px_170px_minmax(0,1fr)] items-center border-b border-border bg-bg text-xs font-medium text-muted">
          <span role="columnheader" class="px-2.5">Time</span><span role="columnheader" class="px-2.5">Channel</span><span role="columnheader" class="px-2.5">Message</span>
        </div>
        <p v-if="!rows.length" class="p-3 text-xs text-muted">{{ filter ? 'No kept message matches that.' : store.subs.length ? 'Nothing has arrived yet.' : 'Listen to a channel to see its messages here.' }}</p>
        <div :style="{ height: `${padTop}px` }" aria-hidden="true" />
        <div v-for="(m, i) in shown" :key="from + i" role="row" :aria-rowindex="from + i + 2" class="grid h-[30px] grid-cols-[120px_170px_minmax(0,1fr)] items-center border-b border-surface-raised">
          <span role="cell" class="px-2.5 font-mono text-xs text-muted">{{ time(m.at_ms ?? 0) }}</span>
          <span role="cell" class="truncate px-2.5 font-mono text-xs text-ansi-cyan" :title="bytesText(m.channel)">{{ bytesText(m.channel) }}</span>
          <span role="cell" class="truncate px-2.5 font-mono text-xs" :title="bytesText(m.payload).slice(0, 2000)">{{ bytesText(m.payload) }}</span>
        </div>
        <div :style="{ height: `${padBottom}px` }" aria-hidden="true" />
      </div>

      <form v-if="!readOnly" class="flex shrink-0 items-center gap-2 border-t border-border bg-surface px-3 py-2.5" @submit.prevent="publish">
        <span class="text-2xs font-medium uppercase tracking-wider text-muted">Publish</span>
        <Input v-model="channel" aria-label="Channel to publish to" class="h-7 w-[180px] font-mono text-xs" placeholder="channel" autocomplete="off" spellcheck="false" />
        <Input v-model="text" aria-label="Message" class="h-7 min-w-0 flex-1 font-mono text-xs" placeholder="Message" autocomplete="off" spellcheck="false" />
        <Button variant="secondary" size="sm" type="submit" :disabled="!channel.trim()">{{ saveLabel === 'Save' ? 'Publish' : 'Review and publish' }}</Button>
      </form>
    </section>
  </div>
</template>
