<script setup lang="ts">
// D15 (E2): a saved job's schedule (hourly, daily, weekly or cron, in this computer's local time), a backup's
// retention, and whether a run that went well notifies too. Rust checks the cron and shows the next runs.
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Checkbox } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import type { Every, JobRow_Serialize as JobRow } from '~/types/bindings'

const props = defineProps<{ job: JobRow }>()
const open = defineModel<boolean>('open', { required: true })
const jobs = useJobsStore()

const DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday']
const KINDS = [['off', 'Off'], ['hour', 'Every hour'], ['day', 'Every day'], ['week', 'Every week'], ['cron', 'Cron']] as const
const f = reactive({ kind: 'off' as 'off' | Every['every'], minute: 0, time: '02:00', weekday: 1, cron: '0 2 * * *', notifyOk: false, keepLast: '', keepDays: '' })
const backup = computed(() => props.job.kind === 'backup')

watch(open, (v) => {
  if (!v) return
  const s = props.job.schedule
  const r = props.job.retention
  const pad = (n: number) => String(n).padStart(2, '0')
  Object.assign(f, {
    kind: s?.every ?? 'off',
    minute: s && 'minute' in s ? s.minute : 0,
    time: s && 'hour' in s ? `${pad(s.hour)}:${pad(s.minute)}` : '02:00',
    weekday: s && 'weekday' in s ? s.weekday : 1,
    cron: s?.every === 'cron' ? s.expr : '0 2 * * *',
    notifyOk: s?.notify_ok ?? false,
    keepLast: r?.keep_last != null ? String(r.keep_last) : '',
    keepDays: r?.keep_days != null ? String(r.keep_days) : '',
  })
  error.value = ''
}, { immediate: true })

const every = computed<Every | null>(() => {
  const [h, m] = f.time.split(':').map(Number)
  switch (f.kind) {
    case 'hour': return { every: 'hour', minute: Number(f.minute) }
    case 'day': return { every: 'day', hour: h ?? 0, minute: m ?? 0 }
    case 'week': return { every: 'week', weekday: Number(f.weekday), hour: h ?? 0, minute: m ?? 0 }
    case 'cron': return { every: 'cron', expr: f.cron }
    default: return null
  }
})
const preview = ref<{ text: string, next: (number | null)[] } | null>(null)
const previewError = ref('')
watch(every, async (e) => {
  preview.value = null
  previewError.value = ''
  if (!e) return
  try { preview.value = await jobs.preview(e) }
  catch (err) { previewError.value = (err as { message?: string })?.message ?? 'That schedule isn\'t valid.' }
}, { immediate: true, deep: true })
const when = (ms: number | null) => (ms ? new Date(ms).toLocaleString(undefined, { weekday: 'short', day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }) : '')
const count = (v: string | number) => (String(v).trim() === '' ? null : Math.max(0, Math.floor(Number(v))))

const error = ref('')
const busy = ref(false)
async function save() {
  busy.value = true
  error.value = ''
  try {
    const e = every.value
    await jobs.setSchedule(props.job.id, e ? { ...e, notify_ok: f.notifyOk } : null, backup.value ? { keep_last: count(f.keepLast), keep_days: count(f.keepDays) } : null)
    open.value = false
  }
  catch (err) { error.value = (err as { message?: string })?.message ?? 'Couldn\'t save the schedule.' }
  busy.value = false
}
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-[520px]" :data-testid="tid('schedule-dialog')">
      <DialogHeader>
        <DialogTitle>Schedule “{{ job.name }}”</DialogTitle>
        <DialogDescription>Runs only while Dalan is open and unlocked, in this computer's time. A run due while Dalan was closed or locked runs once at the next unlock.</DialogDescription>
      </DialogHeader>
      <div class="flex flex-col gap-3 text-sm">
        <div role="radiogroup" aria-label="Runs" class="flex flex-wrap gap-3">
          <label v-for="[k, l] in KINDS" :key="k" class="flex items-center gap-1.5"><input v-model="f.kind" type="radio" name="schedule-kind" :value="k" class="accent-primary">{{ l }}</label>
        </div>
        <label v-if="f.kind === 'hour'" class="flex items-center gap-2">At minute <Input v-model.number="f.minute" type="number" min="0" max="59" class="h-8 w-20" aria-label="Minute past the hour" /></label>
        <div v-if="f.kind === 'day' || f.kind === 'week'" class="flex items-center gap-2">
          <select v-if="f.kind === 'week'" v-model.number="f.weekday" class="h-8 rounded-md border border-border-input bg-bg px-2" aria-label="Weekday">
            <option v-for="(d, i) in DAYS" :key="d" :value="i">{{ d }}</option>
          </select>
          <label class="flex items-center gap-2">at <input v-model="f.time" type="time" class="h-8 rounded-md border border-border-input bg-bg px-2" aria-label="Time"></label>
        </div>
        <label v-if="f.kind === 'cron'" class="flex flex-col gap-1">
          <span class="text-xs text-muted">Minute, hour, day of month, month, weekday (0 is Sunday)</span>
          <Input v-model="f.cron" class="h-8 font-mono" aria-label="Cron expression" placeholder="0 2 * * 1-5" />
        </label>
        <p v-if="previewError" role="alert" class="text-xs text-danger">{{ previewError }}</p>
        <p v-else-if="preview" class="text-xs text-muted" data-part="preview">{{ preview.text }}. Next: {{ preview.next.map(when).join(' · ') }}</p>
        <p v-if="job.kind === 'copy' && f.kind !== 'off'" class="text-xs text-warning">A scheduled copy runs without asking: saving this schedule is the confirmation. Its target is checked again before every run.</p>
        <fieldset v-if="backup" class="flex flex-col gap-2 rounded-md border border-border p-3">
          <legend class="px-1 text-xs font-medium">Keep</legend>
          <label class="flex items-center gap-2">the last <Input v-model="f.keepLast" type="number" min="1" class="h-8 w-20" aria-label="Number of backups to keep" placeholder="all" /> backups</label>
          <label class="flex items-center gap-2">backups from the last <Input v-model="f.keepDays" type="number" min="1" class="h-8 w-20" aria-label="Days of backups to keep" placeholder="all" /> days</label>
          <p class="text-xs text-muted">Only this job's own files in its folder are removed, and only when every rule set here allows it. The newest always stays.</p>
        </fieldset>
        <label v-if="f.kind !== 'off'" class="flex items-center gap-2"><Checkbox v-model="f.notifyOk" />Also notify when a run goes well</label>
        <p class="text-xs text-muted">A failed run always shows a notification and a dot on Jobs.</p>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
      </div>
      <DialogFooter>
        <Button variant="secondary" @click="open = false">Cancel</Button>
        <Button :loading="busy" :disabled="!!previewError" :data-testid="tid('schedule-save')" @click="save">Save</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
