import { defineStore } from 'pinia'
import { toast } from 'vue-sonner'
import type { GenerateInput, IdentityInput, IdentitySummary, ImportInput, KeySummary, KeyUpdate } from '~/types/bindings'
import { CommandError, useTauri } from '~/composables/useTauri'

/** What a dialog needs to show after a failed save: the message, and whether to ask for a passphrase. */
export interface SaveResult { error: string, needsPassphrase?: boolean }

const COPY: Record<string, string> = {
  invalid_key: 'That doesn\'t look like a private key Dalan can read.',
  wrong_password: 'That passphrase didn\'t work.',
}

/** S05-S07: keys and identities. Only public parts and flags are ever held here. */
export const useKeychainStore = defineStore('keychain', () => {
  const api = useTauri()
  const keys = ref<KeySummary[]>([])
  const identities = ref<IdentitySummary[]>([])
  const loaded = ref(false)

  const keyLabel = (id: string | null | undefined) => keys.value.find(k => k.id === id)?.label ?? null

  function fail(e: unknown): SaveResult {
    if (e instanceof CommandError) {
      if (e.code === 'needs_passphrase') return { error: 'This key is protected by a passphrase.', needsPassphrase: true }
      return { error: COPY[e.code] ?? (e.code === 'validation' ? e.message : 'Something went wrong. Try again.') }
    }
    return { error: 'Something went wrong. Try again.' }
  }

  async function refresh() {
    try {
      const [k, i] = await Promise.all([api.keysList(), api.identitiesList()])
      keys.value = k
      identities.value = i
      loaded.value = true
    }
    catch {
      toast.error('Something went wrong. Try again.')
    }
  }

  /** Each returns `{ error: '' }` on success. */
  async function run(fn: () => Promise<unknown>, done = 'Saved.'): Promise<SaveResult> {
    try {
      await fn()
    }
    catch (e) {
      return fail(e)
    }
    await refresh()
    toast(done)
    return { error: '' }
  }

  const generate = (input: GenerateInput) => run(() => api.keyGenerate(input), 'Key saved.')
  const importKey = (input: ImportInput) => run(() => api.keyImport(input), 'Key saved.')
  const updateKey = (input: KeyUpdate) => run(() => api.keyUpdate(input))
  const setCertificate = (id: string, certificate: string | null, path: string | null) => run(() => api.keySetCertificate(id, certificate, path), certificate || path ? 'Certificate attached.' : 'Certificate removed.')
  const saveIdentity = (input: IdentityInput) => run(() => api.identitySave(input))

  async function removeKey(k: KeySummary) {
    const r = await run(() => api.keyDelete(k.id), `Deleted ${k.label}.`)
    if (r.error) toast.error(r.error)
  }
  async function removeIdentity(i: IdentitySummary) {
    const r = await run(() => api.identityDelete(i.id), `Deleted ${i.label}.`)
    if (r.error) toast.error(r.error)
  }

  async function copyPublic(k: KeySummary) {
    try {
      await navigator.clipboard.writeText(k.public_key)
      toast('Public key copied.')
    }
    catch {
      toast.error('Couldn\'t copy. Select and copy it by hand.')
    }
  }

  function clear() {
    keys.value = []
    identities.value = []
    loaded.value = false
  }

  return { keys, identities, loaded, keyLabel, refresh, generate, importKey, updateKey, setCertificate, saveIdentity, removeKey, removeIdentity, copyPublic, clear, pickFile: api.pickFile, keyExportToHost: api.keyExportToHost, sessionClose: api.sessionClose, sessionRespond: api.sessionRespond }
})
