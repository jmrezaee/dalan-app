import type { FsNode } from '../model'

/** Every timestamp in a scenario is relative to this fixed moment, which `installFakeBackend` also sets as the page clock. */
export const NOW = Date.UTC(2026, 10, 6, 15, 0, 0)

export const file = (name: string, size: number, mtimeAgo = 3600_000, mode = 0o644) => ({ name, kind: 'file' as const, size, mtime: NOW - mtimeAgo, mode })
export const dir = (name: string, children: FsNode[] = [], mode = 0o755) => ({ name, kind: 'dir' as const, size: 0, mtime: NOW - 86_400_000, mode, children })

