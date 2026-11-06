<script setup lang="ts">
// S02. Click opens the group. Hosts dropped on it move in (F04).
import { Ellipsis, Folder, Pencil, Trash2 } from '@lucide/vue'
import { cn } from '@/lib/utils'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'

defineProps<{ label: string; hosts: number; groups?: number; dropTarget?: boolean }>()
defineEmits<{ open: []; edit: []; delete: []; drop: [event: DragEvent]; dragover: [event: DragEvent]; dragleave: [] }>()
</script>

<template>
  <div
    tabindex="0"
    role="button"
    :aria-label="`Open ${label}`"
    :class="cn('group flex cursor-pointer items-center gap-3 rounded-lg border border-border bg-surface p-3 hover:border-muted/40 hover:bg-surface-raised', dropTarget && 'border-accent bg-accent/10')"
    @click="$emit('open')"
    @keydown.enter="$emit('open')"
    @dragover="$emit('dragover', $event)"
    @dragleave="$emit('dragleave')"
    @drop.prevent="$emit('drop', $event)"
  >
    <Folder class="size-5 shrink-0 text-accent" aria-hidden="true" />
    <div class="min-w-0 flex-1">
      <div class="truncate text-base">{{ label }}</div>
      <div class="text-xs text-muted">
        {{ hosts }} {{ hosts === 1 ? 'host' : 'hosts' }}<template v-if="groups"> · {{ groups }} {{ groups === 1 ? 'group' : 'groups' }}</template>
      </div>
    </div>
    <DropdownMenu>
      <DropdownMenuTrigger as-child>
        <IconButton label="Group actions" :tip="false" size="xs" class="opacity-0 group-hover:opacity-100 focus-visible:opacity-100 data-[state=open]:opacity-100" @click.stop><Ellipsis /></IconButton>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem @select="$emit('edit')"><Pencil />Edit</DropdownMenuItem>
        <DropdownMenuItem class="text-danger" @select="$emit('delete')"><Trash2 />Delete</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  </div>
</template>
