<script setup lang="ts">
// S24. First-run (password + confirm + strength) and unlock variants. Logic arrives in P2.
import { TriangleAlert } from '@lucide/vue'
import { cn } from '@/lib/utils'

const props = defineProps<{
  mode: 'create' | 'unlock'
  error?: string
  capsLock?: boolean
  strength?: number
  loading?: boolean
  cooldown?: number
}>()
defineEmits<{ submit: [] }>()
const password = defineModel<string>('password', { default: '' })
const confirm = defineModel<string>('confirm', { default: '' })
const bars = [0, 1, 2, 3, 4]
const barTone = (i: number) => (i >= (props.strength ?? 0) ? 'bg-surface-hover' : (props.strength ?? 0) <= 2 ? 'bg-danger-solid' : 'bg-success')
</script>

<template>
  <form class="flex w-[380px] flex-col gap-4 rounded-xl border border-border bg-surface p-8" @submit.prevent="$emit('submit')">
    <DalanMark class="size-10" />
    <div>
      <h1 class="text-2xl">{{ mode === 'create' ? 'Set up Dalan' : 'Unlock Dalan' }}</h1>
      <p v-if="mode === 'create'" class="mt-1.5 text-muted">Choose a master password. It encrypts your keys and passwords on this computer.</p>
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
    <p v-if="capsLock" class="flex items-center gap-1.5 text-xs text-warning"><TriangleAlert class="size-3.5" />Caps Lock is on</p>
    <p v-if="mode === 'create'" class="rounded-md bg-warning/10 px-3 py-2.5 text-xs text-warning">There's no way to recover this password. Keep it in your password manager.</p>
    <Button size="lg" type="submit" :loading="loading" :disabled="!!cooldown">
      {{ cooldown ? `Try again in ${cooldown}s` : mode === 'create' ? 'Create vault' : 'Unlock' }}
    </Button>
  </form>
</template>
