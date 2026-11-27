// Restore any dump (D12, S48, F31). The fake reads no file: a pick returns a made-up dump for the tab's engine,
// chosen by the model's `restoreNext` (clean, hostile, clash, slow). The gate is the real one's rules: Read-only
// refuses, an existing production database needs Open, an existing database is typed, a new one is a confirm
// (typed on production).
import { fail, type Handlers } from '../types'
import type { DumpInfo_Serialize, RestorePlan_Serialize } from '../../../app/types/bindings'

type Keys = 'restorePick' | 'restorePrepare' | 'restoreCommit' | 'restoreForget'

const held = new Map<string, { connId: string, typed: string, target: string, newDb: boolean, slow: boolean }>()
const stops = new Set<string>()
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))

function dumpFor(engine: string, kind: string, folder: boolean): DumpInfo_Serialize {
  const mongo = engine === 'mongodb'
  const my = engine === 'mysql' || engine === 'mariadb'
  const base: DumpInfo_Serialize = {
    id: `dump-${Date.now()}`,
    file_name: mongo ? (folder ? 'shop-dump' : 'shop.archive.gz') : my ? 'shop.sql.gz' : folder ? 'shop.dir' : 'shop-2026-11-20.dump',
    bytes: 18 * 2 ** 20,
    format: mongo ? (folder ? 'mongo_folder' : 'mongo_archive') : my ? 'my_sql' : folder ? 'pg_directory' : 'pg_custom',
    format_label: mongo ? (folder ? 'MongoDB dump folder' : 'MongoDB archive') : my ? 'MySQL SQL script' : folder ? 'PostgreSQL directory dump' : 'PostgreSQL custom archive',
    engine: mongo ? 'mongodb' : my ? 'mysql' : 'postgres',
    mariadb: false,
    guessed: false,
    compression: mongo && !folder ? 'gzip' : my ? 'gzip' : null,
    made_by: mongo ? 'mongodump 100.9.4' : my ? 'MySQL dump 10.13 Distrib 8.4.2' : 'pg_dump 16.4',
    server_version: mongo ? 'MongoDB 7.0.14' : my ? '8.4.2' : 'PostgreSQL 16.4',
    source_db: 'shop',
    created: mongo || my ? null : '2026-11-20 02:00',
    encoding: mongo ? null : 'UTF8',
    databases: mongo ? ['shop'] : [],
    objects: mongo
      ? [{ kind: 'collection', database: 'shop', name: 'customers', rows: 3, rows_exact: true }, { kind: 'collection', database: 'shop', name: 'orders', rows: 1200, rows_exact: true }, { kind: 'view', database: 'shop', name: 'big_orders', rows: null, rows_exact: true }]
      : [
          { kind: 'table', database: null, name: my ? 'customers' : 'public.customers', rows: my ? 50 : null, rows_exact: false },
          { kind: 'table', database: null, name: my ? 'orders' : 'public.orders', rows: my ? 1200 : null, rows_exact: false },
          { kind: 'view', database: null, name: my ? 'big_orders' : 'public.big_orders', rows: null, rows_exact: false },
          { kind: 'function', database: null, name: 'public.order_count(integer)', rows: null, rows_exact: false },
        ],
    objects_more: 0,
    notes: mongo
      ? [{ code: 'indexes', text: '4 index definitions; mongorestore rebuilds them after the documents.' }]
      : [{ code: 'owners', text: 'Objects are owned by app_owner. Dalan skips ownership and privileges by default, so everything belongs to the user you sign in as.' }, { code: 'extensions', text: 'Needs the extensions pgcrypto. Dalan checks the target can install them.' }],
    refused: null,
    needs_upload: folder,
  }
  if (kind === 'hostile') {
    return { ...base, format: mongo ? base.format : my ? 'my_sql' : 'pg_plain', format_label: my ? 'MySQL SQL script' : 'PostgreSQL SQL script', file_name: 'evil.sql', refused: 'Line 18,204 has a client command (`\\!`), which runs a program on the host. Dalan won\'t restore this file.' }
  }
  return base
}

export const restore: Pick<Handlers, Keys> = {
  restorePick: (a, { m }) => {
    const c = m.sqlConns.get(a.connId)
    const db = m.dbs.find(d => d.id === c?.db)
    if (!db) fail('not_found', 'That database connection is closed.')
    return dumpFor(db.engine, m.restoreNext, a.folder)
  },
  restorePrepare: (a, { m }) => {
    const db = m.dbs.find(d => d.id === m.sqlConns.get(a.connId)?.db)
    if (!db) fail('not_found', 'That database connection is closed.')
    const o = a.options
    const newDb = (o.new_database ?? '').trim()
    if (db.write_mode === 'read_only') fail('db_read_only_mode', `${db.name} is read-only in Dalan. Nothing was sent.`)
    if (!newDb && db.environment === 'production' && db.write_mode !== 'open') fail('db_read_only_mode', `${db.name} is a production database. To restore into it, switch it to Open first (that needs the master password), or restore into a new database.`)
    if (newDb === 'shop') fail('validation', 'A database named shop already exists on this server. Pick another name.')
    const target = newDb || db.default_database || 'app'
    const clash = m.restoreNext === 'clash' && !newDb
    const typed = !newDb || db.environment === 'production'
    const plan: RestorePlan_Serialize = {
      pending_id: null,
      blocked: clash && o.existing === 'stop' ? `2 of the dump's tables already exist in ${target}: public.customers, public.orders. Choose “Drop and replace” or “Add rows”, or restore into a new database.` : null,
      level: typed ? 'typed' : 'confirm',
      typed_name: newDb || db.name,
      saved_name: db.name,
      environment: db.environment,
      target_db: target,
      new_database: !!newDb,
      engine: db.engine,
      tool: db.engine === 'mongodb' ? 'mongorestore' : db.engine === 'postgres' ? 'pg_restore' : 'mysql',
      tool_version: db.engine === 'mongodb' ? 'mongorestore version: 100.9.4' : db.engine === 'postgres' ? 'pg_restore (PostgreSQL) 16.4' : 'mysql  Ver 8.4.2',
      runs_on: 'db-primary',
      replaces: clash && o.existing === 'replace' ? ['public.customers', 'public.orders'] : [],
      clashes: clash && o.existing === 'stop' ? ['public.customers', 'public.orders'] : [],
      skipped: o.skip_owner_acl ? ['Ownership and privileges (everything belongs to the user you sign in as)'] : [],
      warnings: [],
      objects: 4,
      rows: 1250,
      bytes: 18 * 2 ** 20,
      upload: false,
      atomic: db.engine === 'postgres' && o.single_transaction,
    }
    if (!plan.blocked) {
      const id = `restore-${held.size + 1}-${Date.now()}`
      held.set(id, { connId: a.connId, typed: plan.typed_name, target, newDb: !!newDb, slow: m.restoreNext === 'slow' })
      plan.pending_id = id
    }
    return plan
  },
  restoreCommit: async (a, { channel, send, tag }) => {
    const h = held.get(a.pendingId)
    if (!h || h.connId !== a.connId) fail('db_confirm_needed', 'That confirmation was already used or isn\'t for this tab. Nothing was sent.')
    if (a.typedName !== null && a.typedName !== h.typed) fail('db_confirm_needed', 'The name doesn\'t match the database\'s name. Nothing was sent.')
    held.delete(a.pendingId)
    const ch = channel(a.progress)
    const total = 18 * 2 ** 20
    const steps = h.slow ? 60 : 3
    for (let i = 1; i <= steps; i++) {
      if (stops.has(a.transferId)) {
        stops.delete(a.transferId)
        tag(`restore:stopped:${h.target}`)
        return { outcome: 'stopped', message: null, target_db: h.target, objects: 1, done: ['table public.customers'], warnings: [], seconds: 2, nothing_changed: true, removed_new_database: h.newDb }
      }
      send(ch, { phase: 'restore', bytes: (total * i) / steps, total, current: i < steps ? 'rows of public.orders' : null, objects: i })
      await sleep(h.slow ? 100 : 10)
    }
    tag(`restore:${h.target}`)
    return {
      outcome: 'done',
      message: null,
      target_db: h.target,
      objects: 12,
      done: ['table public.customers', 'table public.orders'],
      warnings: [{ text: 'ERROR: extension "pgcrypto" is not available', count: 1 }, { text: 'ERROR: role "app_owner" does not exist', count: 214 }],
      seconds: 4,
      nothing_changed: false,
      removed_new_database: false,
    }
  },
  restoreForget: () => null,
}

/** Stop for a running fake restore (`sqlTransferStop` with its transfer id). */
export function stopRestore(id: string) {
  stops.add(id)
}
