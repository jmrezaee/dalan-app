<script setup lang="ts">
// S24. First-run (password + confirm + strength) and unlock variants.
import { TriangleAlert } from '@lucide/vue'
import { cn } from '@/lib/utils'

const props = defineProps<{
  mode: 'create' | 'unlock'
  error?: string
  capsLock?: boolean
  strength?: number
  loading?: boolean
  cooldown?: number
  /** Change the number to replay the wrong-password shake. */
  shake?: number
  autoLockMinutes?: number
}>()
defineEmits<{ submit: [] }>()
const password = defineModel<string>('password', { default: '' })
const confirm = defineModel<string>('confirm', { default: '' })
const bars = [0, 1, 2, 3, 4]
// Replay the shake in place. Re-mounting the form would drop keyboard focus.
const form = ref<HTMLFormElement | null>(null)
watch(() => props.shake, () => {
  const el = form.value
  if (!el) return
  el.classList.remove('animate-shake')
  void el.offsetWidth // force a reflow so the animation restarts
  el.classList.add('animate-shake')
})
// Caps Lock is read from the keyboard events, so it only shows once the user has typed.
const caps = ref(false)
const onKey = (e: KeyboardEvent) => { caps.value = e.getModifierState?.('CapsLock') ?? false }
const barTone = (i: number) => (i >= (props.strength ?? 0) ? 'bg-surface-hover' : (props.strength ?? 0) <= 2 ? 'bg-danger-solid' : 'bg-success')
</script>

<template>
  <form
    ref="form"
    class="flex w-[380px] flex-col gap-4 rounded-xl border border-border bg-surface p-8"
    @submit.prevent="$emit('submit')"
    @keydown="onKey"
    @keyup="onKey"
  >
    <DalanMark class="size-10" />
    <div>
      <h1 class="text-2xl">{{ mode === 'create' ? 'Set up Dalan' : 'Unlock Dalan' }}</h1>
      <p v-if="mode === 'create'" class="mt-1.5 text-muted">Choose a master password. It encrypts your keys and passwords on this computer. There's no way to recover it, so keep it somewhere safe.</p>
    </div>
    <div class="flex flex-col gap-1.5">
      <Label for="mp">Master password</Label>
      <PasswordInput id="mp" v-model="password" :invalid="!!error" />
    </div>
    <div v-if="mode === 'create'" class="flex gap-1" :aria-label="`Password strength: ${strength ?? 0} of 5`">
      <span v-for="i in bars" :key="i" :class="cn('h-1 flex-1 rounded-sm', barTone(i))" />
    </div>
    <div v-if="mode === 'create'" class="flex flex-col gap-1.5">
      <Label for="mp2">Confirm password</Label>
      <PasswordInput id="mp2" v-model="confirm" />
    </div>
    <p v-if="error" class="-mt-2 text-xs text-danger" role="alert">{{ error }}</p>
    <p v-if="capsLock || caps" class="flex items-center gap-1.5 text-xs text-warning"><TriangleAlert class="size-3.5" />Caps Lock is on</p>
    <Button size="lg" type="submit" :loading="loading" :disabled="!!cooldown">
      {{ cooldown ? `Try again in ${cooldown}s` : mode === 'create' ? 'Create vault' : 'Unlock' }}
    </Button>
    <p v-if="mode === 'unlock' && autoLockMinutes" class="text-center text-xs text-muted">Locks again after {{ autoLockMinutes }} minutes idle</p>
  </form>
</template>
