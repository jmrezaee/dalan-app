<script setup lang="ts">
// S05. Never shows secret material: only the public fingerprint.
import { Copy, KeyRound, Lock } from '@lucide/vue'

defineProps<{ label: string; type: string; fingerprint: string; passphraseSaved?: boolean; certificate?: boolean }>()
defineEmits<{ copyPublic: [] }>()
</script>

<template>
  <div class="flex gap-3 rounded-lg border border-border bg-surface p-3 hover:border-muted/40 hover:bg-surface-raised">
    <div class="grid size-8 shrink-0 place-items-center rounded-md bg-surface-hover"><KeyRound class="size-4" aria-hidden="true" /></div>
    <div class="min-w-0 flex-1">
      <div class="flex items-center gap-2">
        <span class="truncate text-base">{{ label }}</span>
        <Badge>{{ type }}</Badge>
        <Badge v-if="certificate">CERT</Badge>
        <Lock v-if="passphraseSaved" class="size-3.5 text-muted" aria-label="Passphrase saved" />
      </div>
      <div class="truncate font-mono text-xs text-muted" :title="fingerprint">{{ fingerprint }}</div>
    </div>
    <IconButton label="Copy public key" size="xs" @click="$emit('copyPublic')"><Copy /></IconButton>
  </div>
</template>
