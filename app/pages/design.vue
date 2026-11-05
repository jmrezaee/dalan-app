<script setup lang="ts">
// Dev-only (M0): every primitive and app component in every state, dark and light side by side.
import { Moon, Sun } from '@lucide/vue'

definePageMeta({ layout: false })
if (!import.meta.dev) throw createError({ statusCode: 404, fatal: true })
const mode = useColorMode()
</script>

<template>
  <div class="min-h-screen bg-chrome p-6 text-text">
    <header class="mb-4 flex items-center justify-between">
      <h1 class="text-2xl">Design</h1>
      <p class="text-muted">Left: dark. Right: light. Floating layers (dialogs, menus, toasts) follow the app theme.</p>
      <Button variant="secondary" @click="mode.preference = mode.value === 'dark' ? 'light' : 'dark'">
        <Sun v-if="mode.value === 'dark'" /><Moon v-else /> App theme: {{ mode.value }}
      </Button>
    </header>
    <div class="grid grid-cols-2 gap-6">
      <section class="dark rounded-xl bg-bg p-5 text-text" aria-label="Dark theme"><DesignShowcase /></section>
      <section class="light rounded-xl bg-bg p-5 text-text" aria-label="Light theme"><DesignShowcase /></section>
    </div>
  </div>
</template>
