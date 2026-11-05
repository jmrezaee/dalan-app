<script setup lang="ts">
// S05. Never shows secret material: only the public fingerprint.
import { Copy, Ellipsis, KeyRound, Lock, Pencil, ServerCog, Trash2 } from '@lucide/vue'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'

defineProps<{ label: string, type: string, fingerprint: string, passphraseSaved?: boolean, encrypted?: boolean, certificate?: boolean }>()
defineEmits<{ copyPublic: [], install: [], edit: [], delete: [] }>()
</script>

<template>
  <div class="group flex gap-3 rounded-lg border border-border bg-surface p-3 hover:border-muted/40 hover:bg-surface-raised">
    <div class="grid size-8 shrink-0 place-items-center rounded-md bg-surface-hover"><KeyRound class="size-4" aria-hidden="true" /></div>
    <div class="min-w-0 flex-1">
      <div class="flex items-center gap-2">
        <span class="truncate text-base">{{ label }}</span>
        <Badge>{{ type }}</Badge>
        <Badge v-if="certificate">CERT</Badge>
        <Lock v-if="encrypted" class="size-3.5 text-muted" :aria-label="passphraseSaved ? 'Passphrase saved' : 'Passphrase protected'" :title="passphraseSaved ? 'Passphrase saved' : 'Asks for its passphrase on every connect'" />
      </div>
      <div class="truncate font-mono text-xs text-muted" :title="fingerprint">{{ fingerprint }}</div>
    </div>
    <IconButton label="Copy public key" size="xs" @click="$emit('copyPublic')"><Copy /></IconButton>
    <DropdownMenu>
      <DropdownMenuTrigger as-child>
        <IconButton label="Key actions" size="xs" class="opacity-0 group-hover:opacity-100 focus-visible:opacity-100 data-[state=open]:opacity-100"><Ellipsis /></IconButton>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem @select="$emit('install')"><ServerCog />Install on a server…</DropdownMenuItem>
        <DropdownMenuItem @select="$emit('edit')"><Pencil />Edit</DropdownMenuItem>
        <DropdownMenuItem class="text-danger" @select="$emit('delete')"><Trash2 />Delete</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  </div>
</template>
