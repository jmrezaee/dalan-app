// The backup check's restore (D17, E7). The fake counts the same rows everywhere except `orders`, which has one
// more row in the source "now" than in the backup.
import { fail, type Handlers } from '../types'

type Keys = 'backupCheckPick' | 'backupVerify'

export const checksFake: Pick<Handlers, Keys> = {
  backupCheckPick: () => ({ id: 'picked-backup', name: 'shop-2026-11-20.dump' }),
  backupVerify: (a, { m, tag }) => {
    const db = m.dbs.find(d => d.id === m.sqlConns.get(a.checkConnId)?.db)
    if (db?.environment === 'production') fail('validation', `This needs a scratch database, and ${db.name} is production. Pick a tab of another server.`)
    tag(`backup-verify:${a.file.kind === 'picked' ? a.file.file_id : `${a.file.sftp_id}:${a.file.path}`}`)
    return {
      read: { ok: true, summary: 'PostgreSQL custom archive (gzip), 14 objects', problem: null },
      rows: [
        { name: 'public.customers', source: 50, restored: 50, same: true },
        { name: 'public.orders', source: 1201, restored: 1200, same: false },
      ],
      same: false,
      restored_on: db?.name ?? '?',
      seconds: 3.2,
    }
  },
}
