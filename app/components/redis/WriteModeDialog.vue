<script setup lang="ts">
// S33: change what Dalan may write to this database. Making a production database less protected needs the
// master password; Rust checks it (the password goes there once and is never kept here).
import type { RedisWriteMode } from '~/types/bindings'
import type { SavedDatabase } from '~/stores/redisDatabases'

const props = defineProps<{ db: SavedDatabase }>()
const open = defineModel<boolean>('open', { required: true })
const databases = useRedisDatabasesStore()

const RANK: Record<RedisWriteMode, number> = { read_only: 2, confirm: 1, open: 0 }
const mode = ref<RedisWriteMode>(props.db.write_mode)
const password = ref('')
const error = ref('')
const busy = ref(false)
watch(open, (v) => {
  if (!v) return
  mode.value = props.db.write_mode
  password.value = ''
  error.value = ''
})
const needsPassword = computed(() => props.db.environment === 'production' && RANK[mode.value] < RANK[props.db.write_mode])

async function submit() {
  if (busy.value || mode.value === props.db.write_mode) return
  if (needsPassword.value && !password.value) {
    error.value = 'Type your master password.'
    return
  }
  busy.value = true
  const res = await databases.setMode(props.db.id, mode.value, needsPassword.value ? password.value : null)
  busy.value = false
  password.value = ''
  if (res.error) error.value = res.error
  else open.value = false
}
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-[440px]" :data-testid="tid('redis-mode-dialog')">
      <DialogHeader>
        <DialogTitle id="redis-mode-title">Writes on {{ db.name }}</DialogTitle>
        <DialogDescription>The check happens in Dalan, before anything leaves this computer. Nothing changes on the server.</DialogDescription>
      </DialogHeader>
      <form class="flex flex-col gap-3" @submit.prevent="submit">
        <WriteModePicker v-model="mode" labelledby="redis-mode-title" />
        <div v-if="needsPassword" class="flex flex-col gap-1.5">
          <label for="redis-mode-password" class="text-sm font-medium">Master password</label>
          <PasswordInput id="redis-mode-password" v-model="password" :invalid="!!error" />
          <p class="text-xs text-muted">Making a production database less protected needs your master password.</p>
        </div>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <DialogFooter>
          <Button variant="secondary" type="button" @click="open = false">Cancel</Button>
          <Button type="submit" :loading="busy" :disabled="mode === db.write_mode">Change mode</Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>
</template>
