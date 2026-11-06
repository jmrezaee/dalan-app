<script setup lang="ts">
// Shows S24 until the vault is unlocked, then renders the app. Logic lives in the vault store.
import { LoaderCircle } from '@lucide/vue'

const vault = useVaultStore()
const password = ref('')
const confirm = ref('')
const localError = ref('')
const shakeKey = ref(0)

onMounted(async () => {
  await vault.refresh()
  vault.startWatching()
})

// A failed attempt clears the field and shakes the card.
watch(() => vault.error, (e) => {
  if (e) {
    shakeKey.value++
    password.value = ''
  }
})
watch(() => vault.phase, () => {
  password.value = ''
  confirm.value = ''
  localError.value = ''
})

// The message goes away as soon as the user edits either field.
watch([password, confirm], () => { localError.value = '' })

async function submit() {
  localError.value = ''
  if (vault.phase === 'needs-setup') {
    if (password.value.length < 8) return void (localError.value = 'Use at least 8 characters.')
    if (password.value !== confirm.value) return void (localError.value = 'Passwords don\'t match.')
    await vault.create(password.value)
  }
  else {
    await vault.unlock(password.value)
  }
}
</script>

<template>
  <slot v-if="vault.phase === 'unlocked'" />
  <main v-else class="grid min-h-screen place-items-center bg-bg p-6 text-text">
    <LoaderCircle v-if="vault.phase === 'loading'" class="size-6 animate-spin text-muted" aria-label="Loading" />
    <div v-else-if="vault.phase === 'unavailable'" class="max-w-sm text-center">
      <h1 class="text-xl">Open Dalan's desktop app</h1>
      <p class="mt-2 text-muted">This page only works inside the app window. Start it with <code class="font-mono">pnpm tauri dev</code>.</p>
    </div>
    <UnlockCard
      v-else
      :key="vault.phase"
      v-model:password="password"
      v-model:confirm="confirm"
      :mode="vault.phase === 'needs-setup' ? 'create' : 'unlock'"
      :error="localError || vault.error"
      :strength="passwordStrength(password)"
      :loading="vault.busy"
      :cooldown="vault.cooldown"
      :shake="shakeKey"
      :auto-lock-minutes="vault.autoLockMinutes"
      @submit="submit"
    />
  </main>
</template>
