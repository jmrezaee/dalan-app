import { defineStore } from 'pinia'
import type { CheckFile, VerifyReport_Serialize as VerifyReport } from '~/types/bindings'
import { useTauri } from '~/composables/useTauri'

const message = (e: unknown, fallback: string) => (e instanceof Error ? e.message : (e as { message?: string })?.message ?? fallback)

/**
 * The backup check's restore (D17, E7): the backup is read back, restored into a scratch database on a server that
 * isn't production, counted against the source and dropped. One check at a time for the whole app.
 */
export const useChecksStore = defineStore('checks', () => {
  const api = useTauri()
  const busy = ref(false)
  const error = ref('')
  const report = ref<VerifyReport | null>(null)
  const picked = ref<{ id: string, name: string } | null>(null)

  async function pick(connId: string) {
    error.value = ''
    try { picked.value = (await api.backupCheckPick(connId)) ?? picked.value }
    catch (e) { error.value = message(e, 'Couldn\'t choose that file.') }
  }
  async function verify(sourceConnId: string, checkConnId: string, file: CheckFile) {
    busy.value = true
    error.value = ''
    report.value = null
    try { report.value = await api.backupVerify(sourceConnId, checkConnId, file) }
    catch (e) { error.value = message(e, 'The check didn\'t finish.') }
    busy.value = false
  }
  function reset() {
    error.value = ''
    report.value = null
    picked.value = null
  }
  return { busy, error, report, picked, pick, verify, reset }
})
