// Small display helpers for the file panes.

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let v = n / 1024
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`
}

/** "Today 14:02", "Nov 2", or "Nov 2, 2025" for a different year. */
export function formatModified(ms: number | null | undefined): string {
  if (ms === null || ms === undefined) return ''
  const d = new Date(ms)
  const now = new Date()
  if (d.toDateString() === now.toDateString()) return `Today ${d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`
  const sameYear = d.getFullYear() === now.getFullYear()
  return d.toLocaleDateString([], sameYear ? { month: 'short', day: 'numeric' } : { year: 'numeric', month: 'short', day: 'numeric' })
}

/** 0o644 -> "-rw-r--r--" (with `d` for folders, `l` for links). */
export function formatMode(mode: number | null | undefined, kind: 'file' | 'dir' | 'other', symlink: boolean): string {
  if (mode === null || mode === undefined) return ''
  const bits = 'rwxrwxrwx'.split('').map((c, i) => (mode & (1 << (8 - i)) ? c : '-')).join('')
  return `${symlink ? 'l' : kind === 'dir' ? 'd' : '-'}${bits}`
}

export function formatSpeed(bytesPerSec: number): string {
  return `${formatBytes(Math.round(bytesPerSec))}/s`
}
