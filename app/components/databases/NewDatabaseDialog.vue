<script setup lang="ts">
// S35 step 1 of 2: what kind of database? Or paste a connection string and skip to the filled-in form (F20).
import { cn } from '@/lib/utils'
import type { Engine } from '~/types/bindings'
import { blankDb } from '~/stores/databases'
import { ENGINES, PICKER_ORDER } from '~/utils/engines'

const databases = useDatabasesStore()
const redisDatabases = useRedisDatabasesStore()

const open = computed({
  get: () => databases.picking,
  set: (v: boolean) => { databases.picking = v; if (!v) url.value = '' },
})
const selected = ref<Engine>('postgres')
const url = ref('')
const error = ref('')
const busy = ref(false)
watch(open, (v) => {
  if (!v) return
  selected.value = 'postgres'
  url.value = ''
  error.value = ''
})

// Arrow keys move the choice, like a radio group.
function onKey(e: KeyboardEvent) {
  const step = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0
  if (!step) return
  e.preventDefault()
  const i = PICKER_ORDER.indexOf(selected.value)
  selected.value = PICKER_ORDER[(i + step + PICKER_ORDER.length) % PICKER_ORDER.length]!
  nextTick(() => (document.querySelector('[data-engine][aria-checked="true"]') as HTMLElement | null)?.focus())
}

async function next() {
  if (busy.value) return
  error.value = ''
  const text = url.value.trim()
  if (!text) return go(selected.value)
  busy.value = true
  const r = await databases.parseUrl(text)
  busy.value = false
  if (!r.fields) { error.value = r.error; return }
  go(r.fields.engine, r.fields, text)
}

function go(engine: Engine, fields: NonNullable<Awaited<ReturnType<typeof databases.parseUrl>>['fields']> | null = null, fromUrl: string | null = null) {
  open.value = false
  databases.pasted = fields
  if (engine === 'redis') {
    redisDatabases.openNew()
    // The Redis form has no connection string field: it gets the parts, and the password is typed there.
    const e = redisDatabases.editing
    if (fields && e && !('has_password' in e)) {
      Object.assign(e, { address: fields.address, port: fields.port ?? 6379, username: fields.username, tls: fields.tls !== 'off', default_db: Number(fields.default_database) || 0 })
    }
    return
  }
  databases.editing = {
    ...blankDb(engine),
    ...(fields ? { address: fields.address, port: fields.port, username: fields.username, default_database: fields.default_database, tls: fields.tls, options: fields.options, from_url: fromUrl } : {}),
  }
}
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-[620px]" :data-testid="tid('new-database-dialog')">
      <DialogHeader>
        <DialogTitle>New database</DialogTitle>
        <DialogDescription>Step 1 of 2 · what kind?</DialogDescription>
      </DialogHeader>
      <form class="flex flex-col gap-3.5" @submit.prevent="next">
        <div role="radiogroup" aria-label="Kind of database" class="grid grid-cols-2 gap-2.5" @keydown="onKey">
          <button
            v-for="e in PICKER_ORDER" :key="e" type="button" role="radio" data-engine :aria-checked="selected === e" :tabindex="selected === e ? 0 : -1"
            :class="cn('flex items-center gap-3 rounded-lg border p-3 text-left outline-none focus-visible:ring-2 focus-visible:ring-accent', selected === e ? 'border-accent bg-accent/5' : 'border-border bg-surface hover:bg-surface-hover')"
            @click="selected = e"
          >
            <EngineBadge :engine="e" large />
            <span class="flex flex-col"><b class="font-semibold">{{ ENGINES[e].label }}</b><span class="text-xs text-muted">{{ ENGINES[e].note }}</span></span>
          </button>
        </div>
        <div class="flex flex-col gap-1.5">
          <Label for="newdb-url" class="text-xs font-normal text-muted">Or paste a connection string (postgres://, mysql://, mongodb+srv://, redis://). Dalan fills the form and moves the password to the vault.</Label>
          <Input id="newdb-url" v-model="url" class="font-mono text-xs" placeholder="postgres://user@host:5432/database" autocomplete="off" spellcheck="false" />
        </div>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <DialogFooter>
          <Button variant="secondary" type="button" @click="open = false">Cancel</Button>
          <Button type="submit" :loading="busy">Continue</Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>
</template>
