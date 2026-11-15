<script setup lang="ts">
// S29: a sorted set's members, highest score first. Writable (R3): add a member, change a score, remove.
import { Plus } from '@lucide/vue'
import { toast } from 'vue-sonner'
import type { RedisBytes } from '~/types/bindings'
import { textBytes } from '~/utils/redisBytes'

const props = defineProps<{ members: { member: RedisBytes, score: string }[], tabKey: string, writable: boolean, saveLabel: string }>()
const store = useRedisStore(props.tabKey)
const rows = computed(() => props.members.map(m => [m.score, m.member]))
const table = ref<{ startAdd: (fixed: (string | null)[]) => void } | null>(null)
const isScore = (s: string) => /^[+-]?inf$/i.test(s.trim()) || (s.trim() !== '' && Number.isFinite(Number(s)))

function save(i: number, values: string[]) {
  const key = store.selectedKey
  const score = (values[0] ?? '').trim()
  if (!key) return
  if (!isScore(score)) return void toast.error('A score is a number, inf or -inf.')
  if (i < 0) {
    if (!values[1]) return void toast.error('A member can\'t be empty.')
    return void store.write({ op: 'zset_add', key, member: textBytes(values[1]), score }, { title: 'Add this member?', action: 'Add member' })
  }
  const m = props.members[i]
  if (m) void store.write({ op: 'zset_add', key, member: m.member, score }, { title: 'Change this score?', action: 'Change' })
}
function remove(i: number) {
  const m = props.members[i]
  if (m && store.selectedKey) void store.write({ op: 'zset_remove', key: store.selectedKey, member: m.member }, { title: 'Remove this member?', action: 'Remove member', danger: true })
}
</script>

<template>
  <div v-if="writable" class="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
    <Button variant="secondary" size="sm" class="ml-auto" @click="table?.startAdd([null, null])"><Plus />Add member</Button>
  </div>
  <ValueRows ref="table" label="Members" item="member" :columns="[{ label: 'Score ↓', width: '120px' }, { label: 'Member' }]" :rows="rows" :edit="writable ? [0] : undefined" :removable="writable" :save-label="saveLabel" @save="save" @remove="remove" />
</template>
