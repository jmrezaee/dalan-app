<script setup lang="ts">
import { TooltipProvider } from '@/components/ui/tooltip'
import { Toaster } from '@/components/ui/sonner'
</script>

<template>
  <TooltipProvider :delay-duration="500" :skip-delay-duration="0">
    <NuxtPage />
    <Toaster />
  </TooltipProvider>
</template>
