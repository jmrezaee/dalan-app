// The key tree of the Redis browser (S28): the scanned keys grouped by the database's separator.
// Built in the UI from what has been scanned so far; nothing here talks to the server.
import type { RedisKeyRow } from '~/types/bindings'
import { bytesId, bytesText } from '~/utils/redisBytes'

export interface TreeFolder {
  kind: 'folder'
  /** The full prefix, separator included: `user:1042:`. Also the folder's id. */
  path: string
  /** The last segment with its separator: `1042:`. */
  name: string
  /** Keys under this folder, at any depth. */
  count: number
  depth: number
  open: boolean
}
export interface TreeKey {
  kind: 'key'
  id: string
  row: RedisKeyRow
  /** What is left of the name after the folders above it: `profile`. */
  name: string
  depth: number
}
export type TreeRow = TreeFolder | TreeKey

interface Node { folders: Map<string, Node>, keys: { name: string, row: RedisKeyRow }[], count: number }
const node = (): Node => ({ folders: new Map(), keys: [], count: 0 })

/** What `treeRows` built last time, so the next page of a scan only adds its own keys. One per key tree. */
export interface TreeCache { keys: RedisKeyRow[], separator: string, root: Node }
export const treeCache = (): TreeCache => ({ keys: [], separator: '', root: node() })

function insert(root: Node, rows: RedisKeyRow[], from: number, separator: string) {
  for (let r = from; r < rows.length; r++) {
    const row = rows[r]!
    const parts = row.key.kind === 'text' && separator ? row.key.text.split(separator) : [bytesText(row.key)]
    let at = root
    at.count++
    // Every part but the last is a folder. An empty part ("a::b", or a trailing separator) stays part of the name.
    for (let i = 0; i < parts.length - 1; i++) {
      const name = parts[i]!
      let next = at.folders.get(name)
      if (!next) {
        next = node()
        at.folders.set(name, next)
      }
      next.count++
      at = next
    }
    at.keys.push({ name: parts[parts.length - 1]!, row })
  }
}

/**
 * The folders and keys of everything scanned. With a cache: the same list is not built again (opening a folder),
 * and a list that only grew at its end (the next page of a scan) only has its new keys added. Anything else
 * (a key deleted, renamed, re-read, or a new scan) builds it again.
 */
function build(keys: RedisKeyRow[], separator: string, cache?: TreeCache): Node {
  if (!cache) {
    const root = node()
    insert(root, keys, 0, separator)
    return root
  }
  const had = cache.keys.length
  const same = cache.separator === separator
  if (same && cache.keys === keys) return cache.root
  // Grew at the end: the rows it had are the same objects, in the same places (checked at both ends).
  const grew = same && had > 0 && keys.length > had && keys[0] === cache.keys[0] && keys[had - 1] === cache.keys[had - 1]
  if (!grew) cache.root = node()
  insert(cache.root, keys, grew ? had : 0, separator)
  cache.keys = keys
  cache.separator = separator
  return cache.root
}

/**
 * The rows to draw, top to bottom, for the folders that are open.
 * - Only readable (text) keys are split; a binary key is one row at the top level.
 * - A folder that holds a single key is not worth a click: that key is shown in its place, with the rest of its name.
 * - Folders come first, then keys, each by name.
 */
export function treeRows(keys: RedisKeyRow[], separator: string, open: ReadonlySet<string>, cache?: TreeCache): TreeRow[] {
  const root = build(keys, separator, cache)

  const out: TreeRow[] = []
  /** The one key under a folder that holds exactly one, with its name from that folder down. */
  const onlyKey = (n: Node, prefix: string): { name: string, row: RedisKeyRow } | null => {
    if (n.count !== 1) return null
    if (n.keys.length === 1) return { name: prefix + n.keys[0]!.name, row: n.keys[0]!.row }
    const [name, child] = [...n.folders][0]!
    return onlyKey(child, `${prefix}${name}${separator}`)
  }
  const walk = (n: Node, path: string, depth: number) => {
    const inline: { name: string, row: RedisKeyRow }[] = []
    for (const [name, child] of [...n.folders].sort(([a], [b]) => a.localeCompare(b))) {
      const single = onlyKey(child, `${name}${separator}`)
      if (single) {
        inline.push(single)
        continue
      }
      const full = `${path}${name}${separator}`
      const isOpen = open.has(full)
      out.push({ kind: 'folder', path: full, name: `${name}${separator}`, count: child.count, depth, open: isOpen })
      if (isOpen) walk(child, full, depth + 1)
    }
    for (const k of [...n.keys, ...inline].sort((a, b) => a.name.localeCompare(b.name))) out.push({ kind: 'key', id: bytesId(k.row.key), row: k.row, name: k.name, depth })
  }
  walk(root, '', 0)
  return out
}

/** The folders above a key, so opening a key can open the way to it: `user:1042:profile` gives `user:`, `user:1042:`. */
export function foldersOf(keyText: string, separator: string): string[] {
  if (!separator) return []
  const parts = keyText.split(separator)
  const out: string[] = []
  let path = ''
  for (let i = 0; i < parts.length - 1; i++) {
    path += parts[i]! + separator
    out.push(path)
  }
  return out
}
