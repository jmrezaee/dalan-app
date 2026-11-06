<script setup lang="ts">
import { TooltipProvider } from '@/components/ui/tooltip'
import { Toaster } from '@/components/ui/sonner'

// The dev-only design page works without a vault, so it can render in a plain browser.
const route = useRoute()
const bypass = computed(() => import.meta.dev && route.path === '/design')
</script>

<template>
  <TooltipProvider :delay-duration="500" :skip-delay-duration="0">
    <NuxtPage v-if="bypass" />
    <VaultGate v-else><NuxtPage /></VaultGate>
    <!-- Bottom centre: the right-hand editor sheets keep their footer buttons uncovered. -->
    <Toaster position="bottom-center" />
  </TooltipProvider>
</template>
