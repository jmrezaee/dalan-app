// SFTP panes, file operations, transfers and "edit a server file". A host's files are a tree in the model;
// "local" is this computer's tree. Transfers copy nodes between trees, so the lists really change.
import type { FsEntry_Serialize } from '../../../app/types/bindings'
import type { FsNode, Model } from '../model'
import { fail, type Ctx, type Handlers } from '../types'

type Keys = 'sftpOpen' | 'sftpRespond' | 'sftpClose' | 'fsHome' | 'fsRealpath' | 'fsList' | 'fsMkdir' | 'fsRename' | 'fsRemove' | 'fsChmod'
  | 'transferStart' | 'transferResolve' | 'transferCancel' | 'editOpen' | 'editUpload' | 'editClose' | 'editApps' | 'editAppForget'

const HOME = '/home/me'
const parts = (path: string) => path.split('/').filter(Boolean)

/** The tree behind an endpoint: "local", or an SFTP connection's host. */
const treeOf = (m: Model, endpoint: string): FsNode[] => {
  const key = endpoint === 'local' ? 'local' : m.sftp.get(endpoint)
  if (!key) fail('not_found', 'That connection is closed.')
  return (m.files[key] ??= [])
}

/** Resolves `.` and `..`, the way a server's realpath does. */
function normalise(path: string): string {
  const out: string[] = []
  for (const p of parts(path)) {
    if (p === '.') continue
    if (p === '..') out.pop()
    else out.push(p)
  }
  return `/${out.join('/')}`
}

function find(root: FsNode[], path: string): FsNode | FsNode[] | undefined {
  let level = root
  let node: FsNode | undefined
  const ps = parts(normalise(path))
  if (!ps.length) return root
  for (const p of ps) {
    node = level.find(n => n.name === p)
    if (!node) return undefined
    level = node.children ?? []
  }
  return node
}

const childrenOf = (root: FsNode[], path: string): FsNode[] => {
  const n = find(root, path)
  if (!n) fail('not_found', `${path} doesn't exist.`)
  if (Array.isArray(n)) return n
  if (n.kind !== 'dir') fail('validation', `${path} is not a folder.`)
  if (n.name === 'private') fail('permission_denied', `You don't have permission to open ${path}.`)
  return (n.children ??= [])
}

const entry = (n: FsNode): FsEntry_Serialize => ({ name: n.name, kind: n.kind, symlink: false, size: n.kind === 'dir' ? null : n.size, mtime: n.mtime, mode: n.mode })
const parentAndName = (root: FsNode[], path: string) => {
  const ps = parts(normalise(path))
  const name = ps.pop()!
  return { list: childrenOf(root, `/${ps.join('/')}`), name }
}
const clone = (n: FsNode): FsNode => ({ ...n, children: n.children?.map(clone) })

const SCRIPT_TYPES = ['command', 'desktop', 'terminal', 'sh', 'bat', 'cmd', 'ps1', 'scpt']
const PROGRAM_TYPES = ['app', 'exe', 'jar', 'pkg', 'dmg', 'msi', 'com', 'scr', 'vbs', 'workflow', 'action']
const extOf = (p: string) => (p.lastIndexOf('.') > p.lastIndexOf('/') + 1 ? p.slice(p.lastIndexOf('.') + 1).toLowerCase() : '')

export const sftp: Pick<Handlers, Keys> = {
  sftpOpen: (a, { m, id }) => {
    const host = m.hosts.find(h => h.id === a.hostId)
    if (!host) fail('not_found', 'That host is gone.')
    const sftpId = id('sftp')
    m.sftp.set(sftpId, host.id)
    m.files[host.id] ??= [{ name: 'home', kind: 'dir', size: 0, mtime: m.now, mode: 0o755, children: [{ name: 'me', kind: 'dir', size: 0, mtime: m.now, mode: 0o755, children: [] }] }]
    // A host key question would arrive on `events` here; tests that need it use the session fake instead.
    return { sftp_id: sftpId, home: HOME, label: host.label }
  },
  sftpRespond: () => null,
  sftpClose: (a, { m }) => {
    m.sftp.delete(a.sftpId)
  },
  fsHome: () => HOME,
  fsRealpath: (a, { m }) => {
    const p = normalise(a.path)
    if (!find(treeOf(m, a.endpoint), p)) fail('not_found', `${a.path} doesn't exist.`)
    return p
  },
  fsList: (a, { m }) => {
    const items = childrenOf(treeOf(m, a.endpoint), a.path)
    return items
      .filter(n => a.showHidden || !n.name.startsWith('.'))
      .sort((x, y) => (x.kind === y.kind ? x.name.localeCompare(y.name) : x.kind === 'dir' ? -1 : 1))
      .map(entry)
  },
  fsMkdir: (a, { m }) => {
    const { list, name } = parentAndName(treeOf(m, a.endpoint), a.path)
    if (list.some(n => n.name === name)) fail('validation', `${name} already exists.`)
    list.push({ name, kind: 'dir', size: 0, mtime: m.now, mode: 0o755, children: [] })
    return null
  },
  fsRename: (a, { m }) => {
    const root = treeOf(m, a.endpoint)
    const from = parentAndName(root, a.from)
    const to = parentAndName(root, a.to)
    const node = from.list.find(n => n.name === from.name)
    if (!node) fail('not_found', `${a.from} doesn't exist.`)
    if (to.list.some(n => n.name === to.name)) fail('validation', `${to.name} already exists.`)
    from.list.splice(from.list.indexOf(node), 1)
    to.list.push({ ...node, name: to.name })
    return null
  },
  fsRemove: (a, { m }) => {
    const { list, name } = parentAndName(treeOf(m, a.endpoint), a.path)
    const i = list.findIndex(n => n.name === name)
    if (i >= 0) list.splice(i, 1)
    return null
  },
  fsChmod: (a, { m }) => {
    const n = find(treeOf(m, a.endpoint), a.path)
    if (n && !Array.isArray(n)) n.mode = a.mode
    return null
  },

  // Transfers: copy each item, ask when the target exists, and tell the pane when everything is done.
  transferStart: (a, { m, id, channel, send }) => {
    const transferId = id('transfer')
    const events = channel(a.events)
    const t = { events, items: a.items as { src_endpoint: string, src_path: string, dst_endpoint: string, dst_dir: string }[], pending: null as number | null, applyAll: null as string | null }
    m.transfers.set(transferId, t)
    if (!m.manualTransfers) setTimeout(() => runTransfer(m, transferId, send), 0)
    return { transfer_id: transferId }
  },
  transferResolve: (a, { m, send }) => {
    const t = m.transfers.get(a.transferId)
    if (!t || t.pending === null) return null
    const item = t.pending
    t.pending = null
    if (a.applyToAll) t.applyAll = a.choice
    runTransfer(m, a.transferId, send, a.choice, item)
    return null
  },
  transferCancel: (a, { m, send }) => {
    const t = m.transfers.get(a.transferId)
    if (t) {
      send(t.events, { type: 'done', cancelled: true })
      m.transfers.delete(a.transferId)
    }
    return null
  },

  editOpen: (a, { m, id, channel }) => {
    const ext = extOf(a.path)
    const chosen = a.openWith === 'choose' || (a.openWith === 'saved' && m.openWith[ext])
    if (PROGRAM_TYPES.includes(ext) || (!chosen && SCRIPT_TYPES.includes(ext))) {
      fail('validation', 'That kind of file would run on your computer if opened. Choose Open with… and pick an editor.')
    }
    if (a.openWith === 'choose') m.openWith[ext] = '/Applications/TextEdit.app'
    const editId = id('edit')
    m.edits.set(editId, { events: channel(a.events), endpoint: a.endpoint, path: a.path })
    return { edit_id: editId }
  },
  editUpload: (a, { m }) => {
    if (!m.edits.has(a.editId)) fail('not_found', 'That edit is closed.')
    if (m.editRemoteChanged && !a.force) return 'remote_changed'
    return 'uploaded'
  },
  editClose: (a, { m }) => {
    m.edits.delete(a.editId)
  },
  editApps: (_a, { m }) => Object.fromEntries(Object.entries(m.openWith).map(([ext, path]) => [ext, path.split('/').pop()!.replace(/\.app$/, '')])),
  editAppForget: (a, { m }) => {
    Reflect.deleteProperty(m.openWith, a.ext)
    return null
  },
}

/** Runs the queue from `from` on. Stops at a conflict until `transferResolve` answers it. */
function runTransfer(m: Model, id: string, send: Ctx['send'], choice?: string, from = 0) {
  const t = m.transfers.get(id)
  if (!t) return
  for (let i = from; i < t.items.length; i++) {
    const it = t.items[i]!
    const src = find(treeOf(m, it.src_endpoint), it.src_path)
    const name = parts(it.src_path).pop() ?? ''
    if (!src || Array.isArray(src)) {
      send(t.events, { type: 'item_failed', item: i, message: `${it.src_path} doesn't exist.` })
      continue
    }
    const dest = childrenOf(treeOf(m, it.dst_endpoint), it.dst_dir)
    const clash = dest.find(n => n.name === name)
    const pick = i === from && choice ? choice : t.applyAll
    if (clash && !pick) {
      t.pending = i
      send(t.events, { type: 'conflict', item: i, name, src_size: src.size, src_mtime: src.mtime, dest_size: clash.size, dest_mtime: clash.mtime })
      return
    }
    if (clash && pick === 'skip') {
      send(t.events, { type: 'item_done', item: i })
      continue
    }
    send(t.events, { type: 'started', item: i, name, total: src.size })
    send(t.events, { type: 'progress', item: i, bytes: src.size, total: src.size })
    const copy = clone(src)
    if (clash && pick === 'replace') dest.splice(dest.indexOf(clash), 1)
    if (clash && pick === 'keep_both') copy.name = `${name} (copy)`
    dest.push(copy)
    send(t.events, { type: 'item_done', item: i })
  }
  send(t.events, { type: 'done', cancelled: false })
  m.transfers.delete(id)
}

