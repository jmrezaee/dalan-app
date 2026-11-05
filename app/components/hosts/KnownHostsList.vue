<script setup lang="ts">
// S14. Fingerprints Dalan has trusted. Forgetting one means the next connection asks again.
import { ShieldCheck, Trash2 } from '@lucide/vue'
import { toast } from 'vue-sonner'
import type { KnownHost } from '~/types/bindings'
import { CommandError, useTauri } from '~/composables/useTauri'
import { buttonVariants } from '@/components/ui/button'

const api = useTauri()
const rows = ref<KnownHost[]>([])
const loaded = ref(false)
const target = ref<KnownHost | null>(null)
const confirmOpen = ref(false)

async function refresh() {
  try {
    rows.value = await api.knownHostsList()
    loaded.value = true
  }
  catch {
    toast.error('Something went wrong. Try again.')
  }
}
onMounted(refresh)
defineExpose({ refresh })

function ask(r: KnownHost) {
  target.value = r
  confirmOpen.value = true
}
async function forget() {
  const r = target.value
  confirmOpen.value = false
  if (!r) return
  try {
    await api.knownHostDelete(r.id)
    toast(`Forgot ${r.host_pattern}.`)
  }
  catch (e) {
    if (!(e instanceof CommandError && e.code === 'not_found')) toast.error('Something went wrong. Try again.')
  }
  await refresh()
}
const date = (ms: number | null) => (ms === null ? '' : new Date(ms).toLocaleDateString([], { year: 'numeric', month: 'short', day: 'numeric' }))
</script>

<template>
  <div class="flex-1 overflow-auto p-6">
    <EmptyState v-if="loaded && !rows.length" :icon="ShieldCheck" title="No trusted servers yet" text="Fingerprints appear here after your first connection." />
    <ul v-else class="flex flex-col gap-1.5" aria-label="Trusted servers">
      <li v-for="r in rows" :key="r.id" class="group flex items-center gap-3 rounded-lg border border-border bg-surface px-3 py-2">
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2"><span class="truncate font-mono text-sm">{{ r.host_pattern }}</span><Badge variant="outline">{{ r.key_type }}</Badge></div>
          <div class="truncate font-mono text-xs text-muted">{{ r.fingerprint }}</div>
        </div>
        <span class="hidden text-xs text-muted sm:block">Added {{ date(r.created_at) }}</span>
        <IconButton :label="`Forget ${r.host_pattern}`" size="xs" @click="ask(r)"><Trash2 /></IconButton>
      </li>
    </ul>

    <AlertDialog v-model:open="confirmOpen">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Forget {{ target?.host_pattern }}?</AlertDialogTitle>
          <AlertDialogDescription>Dalan will ask you to check its fingerprint again the next time you connect.</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction :class="buttonVariants({ variant: 'destructive' })" @click="forget">Forget key</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
