import { defineStore } from 'pinia'
import { toast } from 'vue-sonner'
import { events } from '~/types/bindings'
import type { SshTarget } from '~/types/bindings'
import { useTauri } from '~/composables/useTauri'
import { findQuickHost } from '~/utils/quickConnect'

/**
 * `ssh://` links from other apps (Windows, when Dalan is the handler). Rust parses them strictly and queues them; the
 * UI only ever asks first: a link never connects by itself, and there is no "don't ask again".
 */
export const useLinksStore = defineStore('links', () => {
  const api = useTauri()
  const hosts = useHostsStore()
  /** Links waiting for an answer, oldest first. The first one is on screen. */
  const pending = ref<SshTarget[]>([])
  let watching = false

  async function take() {
    const links = await api.linksTake().catch(() => [])
    for (const l of links) {
      if (l) pending.value.push(l)
      else toast.error('This link isn\'t a valid ssh:// address.')
    }
  }

  /** Listens once, and picks up what arrived before the app was ready (a cold start). A plain browser has no events. */
  function watch() {
    if (!watching) {
      watching = true
      events.linkReceived.listen(() => { void take() }).catch(() => {})
    }
    void take()
  }

  const current = computed(() => pending.value[0] ?? null)
  const query = computed(() => (current.value ? { username: current.value.user ?? null, address: current.value.host, port: current.value.port } : null))
  /** The saved host this link points at, if any: shown by name, and used as it is. */
  const savedHost = computed(() => (query.value ? findQuickHost(hosts.hosts, query.value) : undefined))
  const label = computed(() => (current.value ? `${current.value.user ? `${current.value.user}@` : ''}${current.value.host.includes(':') ? `[${current.value.host}]` : current.value.host}:${current.value.port}` : ''))

  async function connect() {
    const q = query.value
    pending.value.shift()
    if (q) await connectAddress(q)
  }
  function cancel() {
    pending.value.shift()
  }

  return { pending, current, savedHost, label, watch, take, connect, cancel }
})
