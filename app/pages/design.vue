<script setup lang="ts">
// Dev-only design page. P1 fills it with every primitive in every state.
definePageMeta({ layout: false })
if (!import.meta.dev) throw createError({ statusCode: 404 })
</script>

<template>
  <main class="min-h-screen bg-bg p-6 text-text">
    <h1 class="text-xl">Design</h1>
    <p class="text-muted">Primitives land in P1.</p>
  </main>
</template>
