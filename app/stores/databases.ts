import { defineStore } from 'pinia'
import type { DatabaseInput_Deserialize, DatabaseRow_Serialize, DbTestResult, DbUrlFields_Serialize, Engine, SavedQuery_Serialize } from '~/types/bindings'
import { useTauri } from '~/composables/useTauri'
import { DEFAULT_PORT } from '~/utils/engines'

export type DbRow = DatabaseRow_Serialize
export type DbForm = DatabaseInput_Deserialize
export type SavedQueryRow = SavedQuery_Serialize

/** A new PostgreSQL, MySQL, MariaDB or MongoDB database's form. New ones start as "Confirm writes". */
export const blankDb = (engine: Exclude<Engine, 'redis'>, hostId: string | null = null): DbForm => ({
  id: null, engine, name: '', environment: 'none', route: hostId ? 'host' : 'direct', host_id: hostId, address: hostId ? '127.0.0.1' : '',
  port: DEFAULT_PORT[engine], default_database: null, username: null, password: null, clear_password: false, tls: 'off', tls_server_name: null,
  write_mode: 'confirm', options: { auth_source: null, replica_set: null, srv: false }, from_url: null,
})

/**
 * S34, S35 and the Saved queries list: every saved database of every engine, and the saved queries. Redis is still
 * opened and edited through its own store (`useRedisDatabasesStore`), which refreshes this one after a change.
 * Passwords never come back from Rust, only `has_password`.
 */
export const useDatabasesStore = defineStore('databases', () => {
  const api = useTauri()
  const rows = ref<DbRow[]>([])
  const loaded = ref(false)
  const saved = ref<SavedQueryRow[]>([])
  const savedLoaded = ref(false)
  /** Which engines the list shows; `null` is all. */
  const engineFilter = ref<'redis' | 'postgres' | 'mysql' | 'mongodb' | null>(null)
  /** S35 step 1 (the engine picker): open or closed. */
  const picking = ref(false)
  /** S35 step 2 for the engines Rust saves here: `null` is closed; a row edits it; a form (no id) is a new one. */
  const editing = ref<DbRow | DbForm | null>(null)
  /** What to say after a paste that filled the form (the password is not shown, only that it was found). */
  const pasted = ref<DbUrlFields_Serialize | null>(null)

  async function refresh() {
    try {
      rows.value = await api.dbList()
    }
    catch {
      rows.value = [] // locked, or a plain browser
    }
    loaded.value = true
  }

  async function refreshSaved() {
    try {
      saved.value = await api.savedList()
    }
    catch {
      saved.value = []
    }
    savedLoaded.value = true
  }

  const message = (e: unknown, fallback: string) => (e instanceof Error ? e.message : (e as { message?: string })?.message ?? fallback)

  /** Saves the form. `masterPassword` is for a save that makes a production database less protected (Rust checks it). */
  async function save(input: DbForm, masterPassword: string | null = null): Promise<{ db: DbRow | null, error: string }> {
    try {
      const db = await api.dbSave(input, masterPassword)
      await refresh()
      return { db, error: '' }
    }
    catch (e) {
      return { db: null, error: message(e, 'Couldn\'t save the database.') }
    }
  }

  async function remove(db: DbRow): Promise<string> {
    try {
      await api.dbDelete(db.id)
      await refresh()
      return ''
    }
    catch (e) {
      return message(e, 'Couldn\'t delete the database.')
    }
  }

  async function test(input: DbForm): Promise<{ result: DbTestResult | null, error: string }> {
    try {
      return { result: await api.dbTest(input), error: '' }
    }
    catch (e) {
      return { result: null, error: message(e, 'The test failed.') }
    }
  }

  /** Reads a connection string. The password stays in the string; the answer only says whether there was one. */
  async function parseUrl(url: string): Promise<{ fields: DbUrlFields_Serialize | null, error: string }> {
    try {
      return { fields: await api.dbParseUrl(url), error: '' }
    }
    catch (e) {
      return { fields: null, error: message(e, 'Couldn\'t read that connection string.') }
    }
  }

  async function removeSaved(q: SavedQueryRow): Promise<string> {
    try {
      await api.savedDelete(q.id)
      await refreshSaved()
      return ''
    }
    catch (e) {
      return message(e, 'Couldn\'t delete the saved query.')
    }
  }

  const count = (engine: Engine) => rows.value.filter(d => d.engine === engine).length
  function openPicker() {
    pasted.value = null
    picking.value = true
  }

  function clear() {
    rows.value = []
    saved.value = []
    loaded.value = false
    savedLoaded.value = false
    editing.value = null
    picking.value = false
    pasted.value = null
    engineFilter.value = null
  }

  return { rows, loaded, saved, savedLoaded, engineFilter, picking, editing, pasted, refresh, refreshSaved, save, remove, test, parseUrl, removeSaved, count, openPicker, clear }
})
