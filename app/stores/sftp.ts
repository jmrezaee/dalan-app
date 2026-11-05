import { defineStore } from 'pinia'
import { toast } from 'vue-sonner'
import type { Choice, FsEntry, HostSummary, PromptKind, SessionEvent, TransferEvent } from '~/types/bindings'
import { CommandError, useTauri } from '~/composables/useTauri'

export interface Pane {
  /** `local` or an sftp id. `''` = nothing chosen yet (an empty pane). */
  endpoint: string
  hostId: string | null
  label: string
  path: string
  entries: FsEntry[]
  selected: string[]
  showHidden: boolean
  loading: boolean
  connecting: boolean
  error: string
  sort: { by: 'name' | 'size' | 'mtime', desc: boolean }
}

export interface TransferItemRow {
  index: number
  name: string
  direction: 'up' | 'down' | 'copy'
  total: number
  bytes: number
  status: 'queued' | 'running' | 'done' | 'failed'
  message: string
  startedAt: number
}

export interface TransferRow {
  id: string
  items: TransferItemRow[]
  conflict: { item: number, name: string, srcSize: number, srcMtime: number | null, destSize: number, destMtime: number | null } | null
  finished: boolean
  cancelled: boolean
  dstPane: number
}

const blankPane = (): Pane => ({ endpoint: '', hostId: null, label: '', path: '', entries: [], selected: [], showHidden: false, loading: false, connecting: false, error: '', sort: { by: 'name', desc: false } })

/** Remote and local paths use different separators; this keeps panes honest about which they are. */
export const isRemote = (p: Pane) => p.endpoint !== 'local' && p.endpoint !== ''
const sep = (p: Pane) => (isRemote(p) || p.path.startsWith('/') ? '/' : '\\')
export const joinPath = (p: Pane, name: string) => `${p.path.replace(/[\\/]+$/, '')}${sep(p)}${name}`.replace(/^$/, '/')
export function parentPath(p: Pane) {
  const s = sep(p)
  const trimmed = p.path.replace(/[\\/]+$/, '')
  const i = trimmed.lastIndexOf(s)
  return i <= 0 ? s : trimmed.slice(0, i)
}

/** S21 / S22: two file panes (each the local disk or a server) and the transfer queue. */
export const useSftpStore = defineStore('sftp', () => {
  const api = useTauri()
  const panes = reactive<[Pane, Pane]>([blankPane(), blankPane()])
  const transfers = ref<TransferRow[]>([])
  /** A host-key or password question from a pane that is connecting. */
  const prompt = ref<{ pane: number, openId: string, id: string, kind: PromptKind } | null>(null)
  /** What is being dragged between panes. */
  const dragging = ref<{ pane: number, names: string[] } | null>(null)

  function message(e: unknown) {
    if (e instanceof CommandError) return e.code === 'validation' || e.code === 'permission_denied' || e.code === 'not_found' || e.code === 'host_unreachable' || e.code === 'auth_failed' || e.code === 'internal' ? e.message : 'Something went wrong. Try again.'
    return 'Something went wrong. Try again.'
  }

  const sorted = (p: Pane) => {
    const dir = p.sort.desc ? -1 : 1
    return [...p.entries].sort((a, b) => {
      if ((a.kind === 'dir') !== (b.kind === 'dir')) return a.kind === 'dir' ? -1 : 1 // folders first
      if (p.sort.by === 'size') return ((a.size ?? 0) - (b.size ?? 0)) * dir
      if (p.sort.by === 'mtime') return ((a.mtime ?? 0) - (b.mtime ?? 0)) * dir
      return a.name.localeCompare(b.name, undefined, { numeric: true }) * dir
    })
  }

  async function load(i: number, path?: string) {
    const p = panes[i]!
    if (!p.endpoint) return
    p.loading = true
    p.error = ''
    try {
      const target = path ?? p.path
      p.entries = await api.fsList(p.endpoint, target, p.showHidden)
      p.path = target
      p.selected = []
    }
    catch (e) {
      p.error = message(e)
    }
    finally {
      p.loading = false
    }
  }

  async function showLocal(i: number) {
    const p = panes[i]!
    closeRemote(p)
    Object.assign(p, { endpoint: 'local', hostId: null, label: 'This computer', entries: [], selected: [], error: '' })
    try {
      p.path = await api.fsHome('local')
    }
    catch {
      p.path = '/'
    }
    await load(i)
  }

  function closeRemote(p: Pane) {
    if (isRemote(p)) api.sftpClose(p.endpoint)
  }

  async function connect(i: number, host: HostSummary) {
    const p = panes[i]!
    const openId = crypto.randomUUID()
    p.connecting = true
    p.error = ''
    try {
      const opened = await api.sftpOpen(openId, host.id, (e: SessionEvent) => {
        if (e.type === 'prompt') prompt.value = { pane: i, openId, id: e.prompt_id, kind: e.kind }
      })
      closeRemote(p)
      Object.assign(p, { endpoint: opened.sftp_id, hostId: host.id, label: opened.label, path: opened.home, entries: [], selected: [] })
      await load(i)
    }
    catch (e) {
      p.error = message(e)
    }
    finally {
      p.connecting = false
      prompt.value = null
    }
  }

  async function answer(accept: boolean, text: string | null, remember: boolean) {
    const q = prompt.value
    if (!q) return
    prompt.value = null
    await api.sftpRespond(q.openId, q.id, { accept, text, remember }).catch(() => {})
  }

  const refresh = (i: number) => load(i)
  const go = (i: number, path: string) => load(i, path)
  const up = (i: number) => load(i, parentPath(panes[i]!))

  function open(i: number, e: FsEntry) {
    if (e.kind === 'dir') load(i, joinPath(panes[i]!, e.name))
  }

  function toggleHidden(i: number) {
    panes[i]!.showHidden = !panes[i]!.showHidden
    load(i)
  }

  async function run(i: number, fn: () => Promise<unknown>, done?: string) {
    try {
      await fn()
      if (done) toast(done)
    }
    catch (e) {
      toast.error(message(e))
    }
    await load(i)
  }

  const mkdir = (i: number, name: string) => run(i, () => api.fsMkdir(panes[i]!.endpoint, joinPath(panes[i]!, name)), 'Folder created.')
  const rename = (i: number, from: string, to: string) => run(i, () => api.fsRename(panes[i]!.endpoint, joinPath(panes[i]!, from), joinPath(panes[i]!, to)), 'Renamed.')
  const chmod = (i: number, name: string, mode: number) => run(i, () => api.fsChmod(panes[i]!.endpoint, joinPath(panes[i]!, name), mode), 'Permissions changed.')
  async function remove(i: number, names: string[]) {
    const p = panes[i]!
    await run(i, async () => {
      for (const n of names) await api.fsRemove(p.endpoint, joinPath(p, n))
    }, names.length === 1 ? `Deleted ${names[0]}.` : `Deleted ${names.length} items.`)
  }

  // ---- transfers (S22)

  function onTransfer(row: TransferRow, e: TransferEvent) {
    const item = 'item' in e ? row.items[e.item] : undefined
    switch (e.type) {
      case 'started':
        if (item) Object.assign(item, { status: 'running', total: e.total ?? 0, startedAt: Date.now() })
        break
      case 'progress':
        if (item) Object.assign(item, { bytes: e.bytes ?? 0, total: e.total ?? 0 })
        break
      case 'conflict':
        row.conflict = { item: e.item, name: e.name, srcSize: e.src_size ?? 0, srcMtime: e.src_mtime ?? null, destSize: e.dest_size ?? 0, destMtime: e.dest_mtime ?? null }
        break
      case 'item_done':
        if (item) Object.assign(item, { status: 'done', bytes: item.total })
        break
      case 'item_failed':
        if (item) Object.assign(item, { status: 'failed', message: e.message })
        break
      case 'done':
        row.finished = true
        row.cancelled = e.cancelled
        row.conflict = null
        load(row.dstPane) // show what arrived
        break
    }
  }

  /** Copies the named entries from one pane into the folder the other pane is showing. */
  async function copy(from: number, names: string[], to = from === 0 ? 1 : 0) {
    const src = panes[from]!
    const dst = panes[to]!
    if (!src.endpoint || !dst.endpoint) return void toast('Choose a place for both panes first.')
    if (!names.length) return
    const direction = src.endpoint === 'local' ? (dst.endpoint === 'local' ? 'copy' : 'up') : (dst.endpoint === 'local' ? 'down' : 'copy')
    const row = reactive<TransferRow>({
      id: '',
      dstPane: to,
      finished: false,
      cancelled: false,
      conflict: null,
      items: names.map((name, index) => ({ index, name, direction, total: 0, bytes: 0, status: 'queued', message: '', startedAt: 0 })),
    })
    transfers.value.unshift(row)
    try {
      const { transfer_id } = await api.transferStart(
        names.map(n => ({ src_endpoint: src.endpoint, src_path: joinPath(src, n), dst_endpoint: dst.endpoint, dst_dir: dst.path })),
        e => onTransfer(row, e),
      )
      row.id = transfer_id
    }
    catch (e) {
      row.finished = true
      row.items.forEach(it => Object.assign(it, { status: 'failed', message: message(e) }))
    }
  }

  const resolveConflict = (row: TransferRow, choice: Choice, all: boolean) => {
    row.conflict = null
    return api.transferResolve(row.id, choice, all).catch(() => {})
  }
  const cancel = (row: TransferRow) => api.transferCancel(row.id).catch(() => {})
  const clearFinished = () => { transfers.value = transfers.value.filter(t => !t.finished) }

  /** Drop on a pane: copy what was dragged from the other pane. */
  function dropOn(to: number) {
    const d = dragging.value
    dragging.value = null
    if (d && d.pane !== to) copy(d.pane, d.names, to)
  }

  function clear() {
    for (const p of panes) closeRemote(p)
    Object.assign(panes[0], blankPane())
    Object.assign(panes[1], blankPane())
    transfers.value = []
    prompt.value = null
  }

  return { panes, transfers, prompt, dragging, sorted, load, showLocal, connect, answer, refresh, go, up, open, toggleHidden, mkdir, rename, chmod, remove, copy, resolveConflict, cancel, clearFinished, dropOn, clear }
})
