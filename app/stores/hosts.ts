import { defineStore } from 'pinia'
import { toast } from 'vue-sonner'
import type { GroupSummary, HostInput, HostSummary } from '~/types/bindings'
import { CommandError, useTauri } from '~/composables/useTauri'

type View = 'grid' | 'list'
const VIEW_KEY = 'dalan.hostsView'

/** S02 / S03. Groups are loaded but only shown from P5. */
export const useHostsStore = defineStore('hosts', () => {
  const api = useTauri()
  const hosts = ref<HostSummary[]>([])
  const groups = ref<GroupSummary[]>([])
  const loaded = ref(false)
  const query = ref('')
  const view = ref<View>((typeof localStorage !== 'undefined' && localStorage.getItem(VIEW_KEY) === 'list') ? 'list' : 'grid')
  /** `null` = sheet closed, `'new'` = creating, a host = editing it. */
  const editing = ref<HostSummary | 'new' | null>(null)

  watch(view, v => localStorage.setItem(VIEW_KEY, v))

  const filtered = computed(() => {
    const q = query.value.trim().toLowerCase()
    if (!q) return hosts.value
    return hosts.value.filter(h => `${h.label} ${h.address} ${h.username ?? ''}`.toLowerCase().includes(q))
  })

  function message(e: unknown) {
    return e instanceof CommandError && e.code === 'validation' ? e.message : 'Something went wrong. Try again.'
  }

  async function refresh() {
    try {
      const t = await api.hostsTree()
      hosts.value = t.hosts
      groups.value = t.groups
      loaded.value = true
    }
    catch (e) {
      toast.error(message(e))
    }
  }

  /** Returns an error string for the form to show, or `''` on success. */
  async function save(input: HostInput) {
    try {
      await api.hostSave(input)
    }
    catch (e) {
      return message(e)
    }
    await refresh()
    toast('Saved.')
    return ''
  }

  async function remove(h: HostSummary) {
    try {
      await api.hostDelete(h.id)
    }
    catch (e) {
      toast.error(message(e))
      return
    }
    await refresh()
    toast(`Deleted ${h.label}.`)
  }

  /** Called when the vault locks: nothing about hosts should linger in memory. */
  function clear() {
    hosts.value = []
    groups.value = []
    loaded.value = false
    editing.value = null
    query.value = ''
  }

  return { hosts, groups, loaded, query, view, editing, filtered, refresh, save, remove, clear }
})
