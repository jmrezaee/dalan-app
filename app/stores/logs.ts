import { defineStore } from 'pinia'
import { toast } from 'vue-sonner'
import type { LogSummary } from '~/types/bindings'
import { CommandError, useTauri } from '~/composables/useTauri'

/** S15: the latest session log per server, plus bookmarked ones. */
export const useLogsStore = defineStore('logs', () => {
  const api = useTauri()
  const logs = ref<LogSummary[]>([])
  const loaded = ref(false)

  const message = (e: unknown) => (e instanceof CommandError && ['validation', 'not_found', 'internal'].includes(e.code) ? e.message : 'Something went wrong. Try again.')

  async function refresh() {
    try {
      logs.value = await api.logsList()
      loaded.value = true
    }
    catch {
      toast.error('Something went wrong. Try again.')
    }
  }

  async function bookmark(l: LogSummary, bookmarked: boolean, comment: string | null) {
    try {
      await api.logBookmark(l.id, bookmarked, comment)
    }
    catch (e) {
      toast.error(message(e))
    }
    await refresh()
  }

  async function remove(l: LogSummary) {
    try {
      await api.logDelete(l.id)
      toast('Log deleted.')
    }
    catch (e) {
      toast.error(message(e))
    }
    await refresh()
  }

  /** Streams a log into a terminal. Returns an error string, or `''`. */
  async function replay(l: LogSummary, write: (b: Uint8Array) => void) {
    try {
      await api.logReplay(l.id, write)
      return ''
    }
    catch (e) {
      return message(e)
    }
  }

  function clear() {
    logs.value = []
    loaded.value = false
  }

  return { logs, loaded, refresh, bookmark, remove, replay, clear }
})
