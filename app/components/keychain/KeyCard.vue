<script setup lang="ts">
// S05. Never shows secret material: only the public fingerprint.
import { BadgeCheck, Copy, Ellipsis, KeyRound, Lock, Pencil, ServerCog, Trash2 } from '@lucide/vue'
import type { MenuItem } from '@/components/common/ItemMenuList.vue'
import { ContextMenu, ContextMenuContent, ContextMenuTrigger } from '@/components/ui/context-menu'
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'

const props = defineProps<{ label: string, type: string, fingerprint: string, passphraseSaved?: boolean, encrypted?: boolean, certificate?: boolean, certificateNote?: string | null }>()
const emit = defineEmits<{ copyPublic: [], install: [], edit: [], delete: [], certificate: [], removeCertificate: [] }>()

// The same list on right-click and in the "…" menu.
const menu = computed<MenuItem[]>(() => [
  { label: 'Copy public key', icon: Copy, onSelect: () => emit('copyPublic') },
  { label: 'Install on a server…', icon: ServerCog, onSelect: () => emit('install') },
  { label: 'Edit', icon: Pencil, onSelect: () => emit('edit') },
  { label: props.certificate ? 'Replace certificate…' : 'Attach certificate…', icon: BadgeCheck, onSelect: () => emit('certificate') },
  ...(props.certificate ? [{ label: 'Remove certificate', onSelect: () => emit('removeCertificate') }] : []),
  { label: 'Delete', icon: Trash2, danger: true, separator: true, onSelect: () => emit('delete') },
])
</script>

<template>
  <ContextMenu>
    <ContextMenuTrigger as-child>
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
      <div v-if="certificateNote" class="truncate text-xs text-muted" :title="certificateNote">{{ certificateNote }}</div>
    </div>
    <IconButton label="Copy public key" size="xs" @click="$emit('copyPublic')"><Copy /></IconButton>
    <DropdownMenu>
      <DropdownMenuTrigger as-child>
        <IconButton label="Key actions" :tip="false" size="xs" class="opacity-0 group-hover:opacity-100 focus-visible:opacity-100 data-[state=open]:opacity-100"><Ellipsis /></IconButton>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <ItemMenuList :items="menu" mode="dropdown" />
      </DropdownMenuContent>
    </DropdownMenu>
  </div>
    </ContextMenuTrigger>
    <ContextMenuContent>
      <ItemMenuList :items="menu" mode="context" />
    </ContextMenuContent>
  </ContextMenu>
</template>
