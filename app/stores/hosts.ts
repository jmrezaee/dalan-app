import { defineStore } from 'pinia'
import { toast } from 'vue-sonner'
import type { GroupInput, GroupSummary, HostInput, HostSummary, Inherited } from '~/types/bindings'
import { CommandError, useTauri } from '~/composables/useTauri'

type View = 'grid' | 'list'
const VIEW_KEY = 'dalan.hostsView'

/** S02 / S03 / S04: hosts, groups, tags, and where you are in the group tree. */
export const useHostsStore = defineStore('hosts', () => {
  const api = useTauri()
  const hosts = ref<HostSummary[]>([])
  const groups = ref<GroupSummary[]>([])
  const loaded = ref(false)
  /** S25 import dialog. */
  const importOpen = ref(false)
  const query = ref('')
  const view = ref<View>((typeof localStorage !== 'undefined' && localStorage.getItem(VIEW_KEY) === 'list') ? 'list' : 'grid')
  /** `null` = the top level. */
  const currentGroupId = ref<string | null>(null)
  const selectedTags = ref<string[]>([])
  /** `null` = sheet closed, `'new'` = creating, an item = editing it. */
  const editing = ref<HostSummary | 'new' | null>(null)
  const editingGroup = ref<GroupSummary | 'new' | null>(null)

  watch(view, v => localStorage.setItem(VIEW_KEY, v))

  const allTags = computed(() => [...new Set(hosts.value.flatMap(h => h.tags))].sort((a, b) => a.localeCompare(b)))
  /** Search or a tag filter searches every host, across groups. */
  const searching = computed(() => query.value.trim() !== '' || selectedTags.value.length > 0)

  const currentGroup = computed(() => groups.value.find(g => g.id === currentGroupId.value) ?? null)
  /** Top level first: [Production, Web] when you are inside Web. */
  const breadcrumb = computed(() => {
    const path: GroupSummary[] = []
    let g = currentGroup.value
    for (let i = 0; g && i < groups.value.length; i++) {
      path.unshift(g)
      g = groups.value.find(x => x.id === g!.parent_id) ?? null
    }
    return path
  })
  const childGroups = computed(() => (searching.value ? [] : groups.value.filter(g => g.parent_id === currentGroupId.value)))

  const visibleHosts = computed(() => {
    if (!searching.value) return hosts.value.filter(h => h.group_id === currentGroupId.value)
    const q = query.value.trim().toLowerCase()
    return hosts.value.filter(h =>
      (!q || `${h.label} ${h.address} ${h.effective_username ?? ''} ${h.tags.join(' ')}`.toLowerCase().includes(q))
      && selectedTags.value.every(t => h.tags.includes(t)),
    )
  })

  const groupLabel = (id: string | null) => groups.value.find(g => g.id === id)?.label ?? null

  /** Groups in tree order with their depth, for a select. `exclude` hides a group and everything below it. */
  function groupOptions(exclude?: string) {
    const out: { id: string, label: string, depth: number }[] = []
    const walk = (parent: string | null, depth: number) => {
      for (const g of groups.value.filter(x => x.parent_id === parent)) {
        if (g.id === exclude) continue
        out.push({ id: g.id, label: g.label, depth })
        walk(g.id, depth + 1)
      }
    }
    walk(null, 0)
    return out
  }

  /** What something placed in `groupId` would inherit ("from Production" hints). */
  function inheritedFor(groupId: string | null): Inherited {
    const g = groups.value.find(x => x.id === groupId)
    if (!g) return { username: null, port: null, password: null, key: null, jump: null, startup_snippet: null }
    // A group can sign in through an identity, which supplies a username, a password and/or a key.
    const ident = g.identity_id ? useKeychainStore().identities.find(i => i.id === g.identity_id) : undefined
    const keyId = g.key_id ?? ident?.key_id ?? null
    return {
      username: g.username ? { value: g.username, group: g.label } : ident ? { value: ident.username, group: g.label } : g.inherited.username,
      port: g.port ? { value: g.port, group: g.label } : g.inherited.port,
      password: g.has_password || ident?.has_password ? g.label : g.inherited.password,
      key: keyId ? { value: keyId, group: g.label } : g.inherited.key,
      jump: g.jump_mode === 'custom' ? { host_ids: g.jump_host_ids, group: g.label } : g.jump_mode === 'none' ? { host_ids: [], group: g.label } : g.inherited.jump,
      startup_snippet: g.startup_snippet_id ? { value: g.startup_snippet_id, group: g.label } : g.inherited.startup_snippet,
    }
  }

  function toggleTag(tag: string) {
    selectedTags.value = selectedTags.value.includes(tag) ? selectedTags.value.filter(t => t !== tag) : [...selectedTags.value, tag]
  }

  function message(e: unknown) {
    return e instanceof CommandError && (e.code === 'validation' || e.code === 'in_use') ? e.message : 'Something went wrong. Try again.'
  }

  async function refresh() {
    try {
      const t = await api.hostsTree()
      hosts.value = t.hosts
      groups.value = t.groups
      // The group you were in may have been deleted.
      if (currentGroupId.value && !t.groups.some(g => g.id === currentGroupId.value)) currentGroupId.value = null
      selectedTags.value = selectedTags.value.filter(tag => allTags.value.includes(tag))
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

  async function saveGroup(input: GroupInput) {
    try {
      await api.groupSave(input)
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

  async function removeGroup(g: GroupSummary) {
    try {
      await api.groupDelete(g.id)
    }
    catch (e) {
      toast.error(message(e))
      return
    }
    await refresh()
    toast(`Deleted ${g.label}.`)
  }

  /** F04: drag hosts into a group, or to the top level with `null`. */
  async function move(ids: string[], groupId: string | null) {
    try {
      await api.hostsMove(ids, groupId)
    }
    catch (e) {
      toast.error(message(e))
      return
    }
    await refresh()
    toast(groupId ? `Moved to ${groupLabel(groupId)}.` : 'Moved to the top level.')
  }

  /** Called when the vault locks: nothing about hosts should linger in memory. */
  function clear() {
    hosts.value = []
    groups.value = []
    loaded.value = false
    editing.value = null
    editingGroup.value = null
    query.value = ''
    currentGroupId.value = null
    selectedTags.value = []
  }

  return {
    hosts, groups, loaded, importOpen, query, view, currentGroupId, selectedTags, editing, editingGroup,
    allTags, searching, currentGroup, breadcrumb, childGroups, visibleHosts,
    groupLabel, groupOptions, inheritedFor, toggleTag, refresh, save, saveGroup, remove, removeGroup, move, clear,
  }
})
