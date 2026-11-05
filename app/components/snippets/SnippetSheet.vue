<script setup lang="ts">
// S12: a name, a package, and the script. "Default servers" are the hosts it is usually run on.
import { Sheet, SheetContent, SheetDescription, SheetTitle } from '@/components/ui/sheet'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import type { SnippetSummary } from '~/types/bindings'

const open = defineModel<boolean>('open', { required: true })
const props = defineProps<{ snippet: SnippetSummary | null }>()
const emit = defineEmits<{ delete: [snippet: SnippetSummary] }>()
const snippets = useSnippetsStore()
const hosts = useHostsStore()

const form = reactive({ label: '', script: '', package_id: null as string | null, targets: [] as string[] })
const error = ref('')
const busy = ref(false)
const current = computed(() => JSON.stringify(form))
const snapshot = ref('')
const dirty = computed(() => current.value !== snapshot.value)

watch(open, (o) => {
  if (!o) return
  const s = props.snippet
  Object.assign(form, { label: s?.label ?? '', script: s?.script ?? '', package_id: s?.package_id ?? null, targets: [...(s?.target_host_ids ?? [])] })
  error.value = ''
  snapshot.value = current.value
})

const packageOptions = computed(() => snippets.packages.map(p => ({ value: p.id, label: p.label })))
const lines = computed(() => Array.from({ length: Math.max(4, form.script.split('\n').length) }, (_, i) => i + 1).join('\n'))
const targetOptions = computed(() => hosts.hosts.filter(h => !form.targets.includes(h.id)).map(h => ({ value: h.id, label: h.label })))
const targetLabel = (id: string) => hosts.hosts.find(h => h.id === id)?.label ?? 'a deleted host'
const adding = ref<string | null>(null)
watch(adding, (v) => {
  if (v) {
    form.targets.push(v)
    nextTick(() => { adding.value = null })
  }
})

async function submit() {
  if (busy.value) return
  busy.value = true
  error.value = await snippets.save({ id: props.snippet?.id ?? null, label: form.label, script: form.script, package_id: form.package_id, target_host_ids: form.targets })
  busy.value = false
  if (!error.value) open.value = false
}
</script>

<template>
  <Sheet v-model:open="open">
    <SheetContent class="w-[440px] max-w-full gap-0 p-0 sm:max-w-[440px]" @keydown.meta.s.prevent="submit" @keydown.ctrl.s.prevent="submit">
      <SheetDescription class="sr-only">A saved script you can run in any session.</SheetDescription>
      <div class="flex h-14 shrink-0 items-center gap-2 border-b border-border px-5">
        <SheetTitle class="text-base">{{ snippet ? 'Edit snippet' : 'New snippet' }}</SheetTitle>
        <span v-if="dirty" class="text-xs text-warning">· Unsaved changes</span>
      </div>
      <form class="flex flex-1 flex-col gap-4 overflow-auto p-5" @submit.prevent="submit">
        <div class="flex flex-col gap-1.5"><Label for="sn-label">Name</Label><Input id="sn-label" v-model="form.label" placeholder="Deploy latest release" autofocus autocomplete="off" /></div>
        <div class="flex flex-col gap-1.5"><Label for="sn-pkg">Package</Label><OptionSelect id="sn-pkg" v-model="form.package_id" :options="packageOptions" none-label="No package" /></div>
        <div class="flex flex-col gap-1.5">
          <Label for="sn-script">Script</Label>
          <div class="flex overflow-hidden rounded-md border border-border-input bg-terminal-bg focus-within:border-accent focus-within:ring-2 focus-within:ring-accent/40">
            <pre aria-hidden="true" class="select-none border-r border-border px-2 py-2 text-right font-mono text-sm leading-5 text-muted">{{ lines }}</pre>
            <Textarea id="sn-script" v-model="form.script" rows="6" spellcheck="false" class="min-h-0 flex-1 resize-y rounded-none border-0 bg-transparent px-2.5 py-2 font-mono text-sm leading-5 focus-visible:ring-0" />
          </div>
          <p class="text-xs text-muted">Runs line by line in the session. Use the host's environment variables for values that change.</p>
        </div>
        <div class="flex flex-col gap-1.5">
          <Label>Default servers</Label>
          <div class="flex min-h-8 flex-wrap items-center gap-1.5 rounded-md border border-border-input bg-bg px-1.5 py-1">
            <TagChip v-for="id in form.targets" :key="id" :label="targetLabel(id)" removable @remove="form.targets = form.targets.filter(x => x !== id)" />
            <span v-if="!form.targets.length" class="px-1 text-sm text-muted">Any server</span>
          </div>
          <OptionSelect v-if="targetOptions.length" v-model="adding" :options="targetOptions" none-label="Add server…" />
        </div>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <button type="submit" class="hidden" tabindex="-1" aria-hidden="true" />
      </form>
      <div class="flex h-14 shrink-0 items-center justify-end gap-2 border-t border-border px-5">
        <Button v-if="snippet" variant="ghost" class="mr-auto text-danger" @click="emit('delete', snippet)">Delete</Button>
        <Button variant="secondary" @click="open = false">Cancel</Button>
        <Button :loading="busy" @click="submit">Save</Button>
      </div>
    </SheetContent>
  </Sheet>
</template>
