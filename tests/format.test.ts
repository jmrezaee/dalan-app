import { describe, expect, it } from 'vitest'
import { formatBytes, formatMode } from '~/utils/format'

describe('format helpers', () => {
  it('formats sizes', () => {
    expect(formatBytes(612)).toBe('612 B')
    expect(formatBytes(2560)).toBe('2.5 KB')
    expect(formatBytes(710 * 1024 * 1024)).toBe('710 MB')
  })
  it('formats permission bits', () => {
    expect(formatMode(0o644, 'file', false)).toBe('-rw-r--r--')
    expect(formatMode(0o750, 'dir', false)).toBe('drwxr-x---')
    expect(formatMode(0o777, 'file', true)).toBe('lrwxrwxrwx')
    expect(formatMode(null, 'file', false)).toBe('')
  })
})
