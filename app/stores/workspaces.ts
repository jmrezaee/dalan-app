import { defineStore } from 'pinia'
import { toast } from 'vue-sonner'
import type { TemplateSummary } from '~/types/bindings'
import { CommandError, useTauri } from '~/composables/useTauri'
import type { Session } from '~/stores/sessions'

/** F09: saved workspace layouts. A template remembers which hosts were open and in what layout mode. */
export const useWorkspacesStore = defineStore('workspaces', () => {
  const api = useTauri()
  const sessions = useSessionsStore()
  const hosts = useHostsStore()
  const templates = ref<TemplateSummary[]>([])
  const loaded = ref(false)

  async function refresh() {
    try {
      templates.value = await api.workspacesList()
      loaded.value = true
    }
    catch {
      toast.error('Something went wrong. Try again.')
    }
  }

  /** Saves the workspace's panes under a name (or updates it when it already came from a template). */
  async function save(ws: Session, name: string): Promise<string> {
    const panes = ws.panes.map(p => ({ host_id: p.hostId, title: p.label }))
    try {
      const id = await api.workspaceSave({ id: ws.templateId || null, name, mode: ws.mode, layout: JSON.stringify({ mode: ws.mode }), panes: JSON.stringify(panes) })
      ws.templateId = id
      ws.label = name
      ws.dirty = false
    }
    catch (e) {
      return e instanceof CommandError && e.code === 'validation' ? e.message : 'Something went wrong. Try again.'
    }
    await refresh()
    toast(`Saved workspace ${name}.`)
    return ''
  }

  /** Opens a template. Hosts that no longer exist are skipped. */
  function open(t: TemplateSummary) {
    let wanted: { host_id: string }[] = []
    try {
      wanted = JSON.parse(t.panes)
    }
    catch { /* an unreadable template opens nothing */ }
    const list = wanted.map(p => hosts.hosts.find(h => h.id === p.host_id)).filter((h): h is NonNullable<typeof h> => !!h)
    if (!list.length) return void toast('None of this template\'s hosts exist any more.')
    if (list.length < wanted.length) toast(`${wanted.length - list.length} host(s) in this template no longer exist.`)
    sessions.openWorkspace(list, { label: t.name, mode: t.mode === 'focus' ? 'focus' : 'split', templateId: t.id })
  }

  async function remove(t: TemplateSummary) {
    try {
      await api.workspaceDelete(t.id)
    }
    catch {
      return void toast.error('Something went wrong. Try again.')
    }
    await refresh()
    toast(`Deleted ${t.name}.`)
  }

  function clear() {
    templates.value = []
    loaded.value = false
  }

  return { templates, loaded, refresh, save, open, remove, clear }
})
