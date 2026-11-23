<script setup lang="ts">
// S47 "MongoShell": one command per line in the familiar db.collection syntax, no JavaScript. Reads answer at once;
// a change is held by Rust and shown here (with how many documents it reaches) before it is sent. Up and Down walk
// the lines typed in this tab.
import { Input } from '@/components/ui/input'

const props = defineProps<{ tabKey: string }>()
const store = useMongoStore(props.tabKey)
const input = ref<HTMLInputElement | null>(null)
const logEnd = ref<HTMLElement | null>(null)
let at = -1
const r = computed(() => store.shellReview)
const open = computed({ get: () => r.value !== null, set: (v: boolean) => { if (!v) store.shellCancel() } })

function onKey(e: KeyboardEvent) {
  const h = store.shellHistory
  if (e.key === 'ArrowUp' && h.length) {
    e.preventDefault()
    at = at < 0 ? h.length - 1 : Math.max(0, at - 1)
    store.shellInput = h[at]!
  }
  else if (e.key === 'ArrowDown' && at >= 0) {
    e.preventDefault()
    at = at + 1
    store.shellInput = at >= h.length ? '' : h[at]!
    if (at >= h.length) at = -1
  }
}
async function run() {
  at = -1
  await store.shellRun()
  await nextTick()
  logEnd.value?.scrollIntoView({ block: 'end' })
  input.value?.focus()
}
watch(() => store.shellReview, async (v) => { if (!v) { await nextTick(); input.value?.focus() } })
onMounted(() => input.value?.focus())
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col" :data-testid="tid('mongo-shell')">
    <div class="min-h-0 flex-1 overflow-auto p-3 font-mono text-xs" tabindex="0" role="log" aria-label="Shell output">
      <p v-if="!store.shellLog.length" class="font-sans text-sm text-muted">Type a command in the familiar db.collection syntax, like <code class="font-mono">db.orders.find({ status: "open" }).limit(5)</code>, or <code class="font-mono">show collections</code>. Reads run at once; changes show what they will do and ask first.</p>
      <div v-for="e in store.shellLog" :key="e.id" class="mb-3" :data-testid="tid('mongo-shell-entry')">
        <div class="text-muted">{{ e.db }}&gt; <span class="text-text">{{ e.line }}</span></div>
        <pre v-for="(b, i) in e.blocks" :key="i" class="m-0 whitespace-pre-wrap break-all">{{ b }}</pre>
        <p v-if="e.note" class="m-0 font-sans text-2xs text-muted">{{ e.note }}</p>
        <p v-if="e.error" role="alert" class="m-0 font-sans text-xs text-danger" :data-testid="tid('mongo-shell-error')">{{ e.error }}</p>
      </div>
      <div ref="logEnd" />
    </div>
    <form class="flex shrink-0 items-center gap-2 border-t border-border px-3 py-2" aria-label="Command" @submit.prevent="run">
      <label class="font-mono text-xs text-muted" for="mg-shell-input">{{ store.shellDb }}&gt;</label>
      <Input id="mg-shell-input" ref="input" v-model="store.shellInput" class="h-8 flex-1 font-mono text-xs" autocomplete="off" spellcheck="false" autocapitalize="off" :disabled="store.shellBusy || !!store.shellReview" :data-testid="tid('mongo-shell-input')" @keydown="onKey" />
    </form>

    <AlertDialog v-model:open="open">
      <AlertDialogContent v-if="r" :class="['gap-3 sm:max-w-[560px]', r.pending.level === 'typed' && 'border-danger']" :data-testid="tid('mongo-shell-review')">
        <AlertDialogHeader>
          <AlertDialogTitle>{{ r.pending.level === 'typed' ? `Dangerous command on ${r.pending.db_name}?` : 'Run this change?' }}</AlertDialogTitle>
          <AlertDialogDescription>
            {{ r.pending.summary }}
            <span v-if="r.pending.environment === 'production'" class="text-warning"> {{ r.pending.db_name }} is a production database.</span>
          </AlertDialogDescription>
        </AlertDialogHeader>
        <pre class="m-0 max-h-24 overflow-auto rounded-md border border-border bg-bg p-2 font-mono text-xs" aria-label="Command">{{ r.line }}</pre>
        <details v-if="r.pending.sample"><summary class="cursor-default text-xs text-muted">Show a sample of what it reaches</summary><pre class="mt-1 max-h-48 overflow-auto rounded-md border border-border bg-bg p-2 font-mono text-xs">{{ r.pending.sample }}</pre></details>
        <label v-if="r.pending.level === 'typed'" class="flex flex-col gap-1 text-xs text-muted">Type {{ r.pending.db_name }} to confirm
          <Input v-model="r.typed" class="font-mono text-xs text-text" autocomplete="off" spellcheck="false" :data-testid="tid('mongo-shell-typed')" @keydown.enter.prevent="store.shellCommit()" />
        </label>
        <p v-if="r.error" role="alert" class="text-xs text-danger">{{ r.error }}</p>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <Button :variant="r.pending.level === 'typed' ? 'destructive' : 'default'" :disabled="r.busy || (r.pending.level === 'typed' && r.typed !== r.pending.db_name)" :data-testid="tid('mongo-shell-confirm')" @click="store.shellCommit()">{{ r.pending.level === 'typed' ? 'Run it anyway' : 'Run' }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
