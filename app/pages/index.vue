<script setup lang="ts">
// Placeholder until the app shell (P7). Proves unlock works and lets you lock again.
import { Lock } from '@lucide/vue'

const vault = useVaultStore()
</script>

<template>
  <main class="flex h-screen flex-col items-center justify-center gap-4 bg-bg text-text">
    <div class="flex items-center gap-2 text-2xl"><DalanMark class="size-8" />Dalan</div>
    <p class="text-muted">Vault unlocked.</p>
    <Button variant="secondary" @click="vault.lock()"><Lock />Lock</Button>
  </main>
</template>
