<script setup lang="ts">
// S15: replays a saved log in a read-only terminal. The log is decrypted in Rust and streamed in.
import { Terminal } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'
import type { LogSummary } from '~/types/bindings'
import { terminalThemes } from '~/utils/terminal-themes'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'

const props = defineProps<{ log: LogSummary | null }>()
const open = defineModel<boolean>('open', { required: true })
const logs = useLogsStore()
const settings = useSettingsStore()
const host = ref<HTMLElement | null>(null)
const error = ref('')
let term: Terminal | undefined

watch(open, async (o) => {
  term?.dispose()
  term = undefined
  error.value = ''
  if (!o || !props.log) return
  await nextTick()
  // The dialog content mounts a tick later than `open` flips.
  await new Promise(r => setTimeout(r, 50))
  if (!host.value) return
  term = new Terminal({
    fontFamily: '"JetBrains Mono Variable", "JetBrains Mono", ui-monospace, monospace',
    fontSize: settings.terminal.font_size,
    lineHeight: 1.2,
    scrollback: 50000,
    disableStdin: true,
    cursorBlink: false,
    convertEol: false,
    theme: terminalThemes[settings.resolvedTheme === 'light' ? 'default-light' : 'default-dark'],
  })
  term.open(host.value)
  error.value = await logs.replay(props.log, b => term?.write(b))
})
onBeforeUnmount(() => term?.dispose())
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-[860px]">
      <DialogHeader>
        <DialogTitle>{{ log?.host_label ?? 'This computer' }}</DialogTitle>
        <DialogDescription>{{ log ? new Date(log.started_at ?? 0).toLocaleString() : '' }}. Read only.</DialogDescription>
      </DialogHeader>
      <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
      <div ref="host" class="h-[420px] overflow-hidden rounded-md border border-border bg-bg p-2" aria-label="Log replay" />
    </DialogContent>
  </Dialog>
</template>
