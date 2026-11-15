import { defineStore } from 'pinia'
import type { RedisDatabaseInput_Deserialize, RedisDatabase_Serialize, RedisTestResult, RedisWriteMode } from '~/types/bindings'
import { useTauri } from '~/composables/useTauri'

export type SavedDatabase = RedisDatabase_Serialize
export type DatabaseForm = RedisDatabaseInput_Deserialize

/** A new database's form. New ones start as "Confirm writes" (replica/redis/spec.md). */
export const blankDatabase = (hostId: string | null = null): DatabaseForm => ({
  id: null, name: '', environment: 'none', route: hostId ? 'host' : 'direct', host_id: hostId, address: hostId ? '127.0.0.1' : '', port: 6379, kind: 'single',
  sentinel_master: null, username: null, password: null, clear_password: false, tls: false, tls_server_name: null, write_mode: 'confirm', default_db: 0, key_separator: ':',
})

/** S26 and S27: the saved Redis databases. Passwords never come back from Rust, only `has_password`. */
export const useRedisDatabasesStore = defineStore('redisDatabases', () => {
  const api = useTauri()
  const dbs = ref<SavedDatabase[]>([])
  const loaded = ref(false)
  /** The editor sheet: `null` is closed; a database edits it; an input (no id) is a new one, maybe with a host pre-filled. */
  const editing = ref<SavedDatabase | DatabaseForm | null>(null)
  /** Text the Databases table is filtered by (the host menu sets it to a host's name). */
  const filter = ref('')
  /** Counts up when something outside the section wants it shown (the host menu, the palette). */
  const showRequests = ref(0)

  async function refresh() {
    try {
      dbs.value = await api.redisDbList()
    }
    catch {
      dbs.value = [] // locked, or a plain browser
    }
    loaded.value = true
  }

  /** Saves the form. Returns the saved row, or the sentence to show under the form. */
  async function save(input: DatabaseForm): Promise<{ db: SavedDatabase | null, error: string }> {
    try {
      const db = await api.redisDbSave(input)
      await refresh()
      return { db, error: '' }
    }
    catch (e) {
      return { db: null, error: e instanceof Error ? e.message : 'Couldn\'t save the database.' }
    }
  }

  async function remove(db: SavedDatabase): Promise<string> {
    try {
      await api.redisDbDelete(db.id)
      await refresh()
      return ''
    }
    catch (e) {
      return e instanceof Error ? e.message : 'Couldn\'t delete the database.'
    }
  }

  /** "Test connection": connects with the form's values and says what answered. */
  async function test(input: DatabaseForm): Promise<{ result: RedisTestResult | null, error: string }> {
    try {
      return { result: await api.redisDbTest(input), error: '' }
    }
    catch (e) {
      return { result: null, error: e instanceof Error ? e.message : 'The test failed.' }
    }
  }

  /** Changes the write mode. Loosening a production database needs the master password, which Rust checks. */
  async function setMode(id: string, mode: RedisWriteMode, masterPassword: string | null): Promise<{ error: string, code: string }> {
    try {
      await api.redisSetMode(id, mode, masterPassword)
      await refresh()
      return { error: '', code: '' }
    }
    catch (e) {
      return { error: e instanceof Error ? e.message : 'Couldn\'t change the mode.', code: (e as { code?: string }).code ?? 'internal' }
    }
  }

  const forHost = (hostId: string) => dbs.value.filter(d => d.host_id === hostId)
  function openNew(hostId: string | null = null) {
    // New ones start with what Settings > Databases says (Confirm writes and ":" unless changed).
    const defaults = useSettingsStore().redis
    editing.value = { ...blankDatabase(hostId), write_mode: defaults.default_mode, key_separator: defaults.key_separator }
    showRequests.value++
  }
  function show(text = '') {
    filter.value = text
    showRequests.value++
  }

  function clear() {
    dbs.value = []
    loaded.value = false
    editing.value = null
    filter.value = ''
  }

  return { dbs, loaded, editing, filter, showRequests, refresh, save, remove, test, setMode, forHost, openNew, show, clear }
})
