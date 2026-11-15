<script setup lang="ts">
// S29: a set's members. Writable (R3): add and remove (a member has nothing to edit).
import { Plus } from '@lucide/vue'
import { toast } from 'vue-sonner'
import type { RedisBytes } from '~/types/bindings'
import { textBytes } from '~/utils/redisBytes'

const props = defineProps<{ members: RedisBytes[], tabKey: string, writable: boolean, saveLabel: string }>()
const store = useRedisStore(props.tabKey)
const rows = computed(() => props.members.map(m => [m]))
const table = ref<{ startAdd: (fixed: (string | null)[]) => void } | null>(null)

function save(_i: number, values: string[]) {
  if (!values[0]) return void toast.error('A member can\'t be empty.')
  if (store.selectedKey) void store.write({ op: 'set_add', key: store.selectedKey, member: textBytes(values[0]) }, { title: 'Add this member?', action: 'Add member' })
}
function remove(i: number) {
  const member = props.members[i]
  if (member && store.selectedKey) void store.write({ op: 'set_remove', key: store.selectedKey, member }, { title: 'Remove this member?', action: 'Remove member', danger: true })
}
</script>

<template>
  <div v-if="writable" class="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
    <span class="text-xs text-muted">Members are unique; adding one twice changes nothing.</span>
    <Button variant="secondary" size="sm" class="ml-auto" @click="table?.startAdd([null])"><Plus />Add member</Button>
  </div>
  <ValueRows ref="table" label="Members" item="member" :columns="[{ label: 'Member' }]" :rows="rows" :removable="writable" :save-label="saveLabel" @save="save" @remove="remove" />
</template>
