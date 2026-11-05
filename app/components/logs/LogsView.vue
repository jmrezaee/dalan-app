<script setup lang="ts">
// S15: each server keeps its latest session here. Bookmark one to keep it, add a note, replay it.
import { Bookmark, Play, ScrollText, Trash2 } from '@lucide/vue'
import type { LogSummary } from '~/types/bindings'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'

const emit = defineEmits<{ settings: [] }>()
const logs = useLogsStore()
const settings = useSettingsStore()
onMounted(logs.refresh)

const replaying = ref<LogSummary | null>(null)
const replayOpen = ref(false)
const play = (l: LogSummary) => { replaying.value = l; replayOpen.value = true }

const size = (n: number | null) => { const b = n ?? 0; return (b < 1024 ? `${b} B` : b < 1024 * 1024 ? `${(b / 1024).toFixed(1)} KB` : `${(b / 1024 / 1024).toFixed(1)} MB`) }
const when = (ms: number | null) => new Date(ms ?? 0).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
const length = (l: LogSummary) => {
  if (!l.ended_at) return 'running'
  const s = Math.max(0, Math.round((l.ended_at - (l.started_at ?? 0)) / 1000))
  return s < 60 ? `${s} s` : `${Math.floor(s / 60)} min`
}
const commentDraft = reactive<Record<string, string>>({})
const saveComment = (l: LogSummary) => {
  const c = (commentDraft[l.id] ?? l.comment ?? '').trim()
  if (c !== (l.comment ?? '')) logs.bookmark(l, l.bookmarked, c || null)
}
</script>

<template>
  <PageToolbar title="Logs" />
  <div class="flex-1 overflow-auto p-6">
    <EmptyState v-if="logs.loaded && !logs.logs.length" :icon="ScrollText" title="No logs yet" text="Each server keeps its latest session here. Bookmark one to keep it.">
      <Button v-if="!settings.logsEnabled" @click="emit('settings')">Turn on session logs</Button>
    </EmptyState>
    <template v-else>
      <p v-if="!settings.logsEnabled" class="mb-3 text-xs text-muted">Session logs are off, so new sessions aren't recorded. <button type="button" class="text-accent underline-offset-2 hover:underline" @click="emit('settings')">Turn on in Settings</button></p>
      <ul class="flex flex-col gap-1.5" aria-label="Session logs">
        <li v-for="l in logs.logs" :key="l.id" class="flex items-center gap-3 rounded-lg border border-border bg-surface px-3 py-2">
          <IconButton :label="l.bookmarked ? 'Remove bookmark' : 'Bookmark'" :pressed="l.bookmarked" size="xs" @click="logs.bookmark(l, !l.bookmarked, l.comment ?? null)">
            <Bookmark :class="cn(l.bookmarked && 'fill-accent text-accent')" />
          </IconButton>
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2"><span class="truncate text-sm font-medium">{{ l.host_label ?? 'This computer' }}</span><span class="text-xs text-muted">{{ when(l.started_at) }} · {{ length(l) }} · {{ size(l.size_bytes) }}</span></div>
            <Input
              :model-value="commentDraft[l.id] ?? l.comment ?? ''"
              :aria-label="`Note for the ${l.host_label ?? 'local'} log`"
              placeholder="Add a note"
              maxlength="500"
              class="mt-1 h-7 border-transparent bg-transparent px-1 text-xs hover:border-border-input focus:border-accent"
              @update:model-value="(v: string | number) => (commentDraft[l.id] = String(v))"
              @blur="saveComment(l)"
              @keydown.enter="($event.target as HTMLInputElement).blur()"
            />
          </div>
          <Button variant="secondary" size="sm" @click="play(l)"><Play />Replay</Button>
          <IconButton label="Delete log" size="xs" @click="logs.remove(l)"><Trash2 /></IconButton>
        </li>
      </ul>
    </template>
  </div>
  <LogReplayDialog v-model:open="replayOpen" :log="replaying" />
</template>
