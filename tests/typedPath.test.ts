import { describe, expect, it } from 'vitest'
import { needsResolving } from '../app/utils/typedPath'

describe('needsResolving', () => {
  it('flags home and dot segments', () => {
    for (const p of ['~', '~/logs', '/srv/../etc', '..', './x', '/a/.']) expect(needsResolving(p), p).toBe(true)
  })
  it('leaves plain paths and dotted names alone', () => {
    for (const p of ['/', '/var/log', '/home/me/.config', '/a/b..c', 'C:\\Users']) expect(needsResolving(p), p).toBe(false)
  })
})
