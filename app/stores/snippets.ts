import { defineStore } from 'pinia'
import { toast } from 'vue-sonner'
import type { PackageSummary, SnippetInput, SnippetSummary } from '~/types/bindings'
import { CommandError, useTauri } from '~/composables/useTauri'

/** S11 / S12: saved scripts. Running one means typing it into a session (the side panel, S20). */
export const useSnippetsStore = defineStore('snippets', () => {
  const api = useTauri()
  const sessions = useSessionsStore()
  const snippets = ref<SnippetSummary[]>([])
  const packages = ref<PackageSummary[]>([])
  const loaded = ref(false)

  const snippetLabel = (id: string | null | undefined) => snippets.value.find(s => s.id === id)?.label ?? null

  async function refresh() {
    try {
      const all = await api.snippetsList()
      snippets.value = all.snippets
      packages.value = all.packages
      loaded.value = true
    }
    catch {
      toast.error('Something went wrong. Try again.')
    }
  }

  /** Returns an error string for the form, or `''` on success. */
  async function run(fn: () => Promise<unknown>, done = 'Saved.') {
    try {
      await fn()
    }
    catch (e) {
      return e instanceof CommandError && e.code === 'validation' ? e.message : 'Something went wrong. Try again.'
    }
    await refresh()
    toast(done)
    return ''
  }

  const save = (input: SnippetInput) => run(() => api.snippetSave(input))
  const duplicate = (s: SnippetSummary) => run(() => api.snippetSave({ id: null, label: `${s.label} copy`, script: s.script, package_id: s.package_id, target_host_ids: s.target_host_ids }), `Copied ${s.label}.`)
  const savePackage = (id: string | null, label: string) => run(() => api.packageSave(id, label))
  async function remove(s: SnippetSummary) {
    const err = await run(() => api.snippetDelete(s.id), `Deleted ${s.label}.`)
    if (err) toast.error(err)
  }
  async function removePackage(p: PackageSummary) {
    const err = await run(() => api.packageDelete(p.id), `Deleted ${p.label}. Its snippets are kept.`)
    if (err) toast.error(err)
  }

  /** Types the script into the active terminal, as if you had pasted it and pressed Enter. */
  function runInActive(s: SnippetSummary) {
    const sess = sessions.activeSession
    if (!sess || sess.kind !== 'session' || sess.status !== 'ready') {
      toast('Open a connection first, then run the snippet in it.')
      return false
    }
    sessions.write(sess, new TextEncoder().encode(`${s.script.replace(/\s+$/, '')}\n`))
    return true
  }

  function clear() {
    snippets.value = []
    packages.value = []
    loaded.value = false
  }

  return { snippets, packages, loaded, snippetLabel, refresh, save, duplicate, savePackage, remove, removePackage, runInActive, clear }
})
