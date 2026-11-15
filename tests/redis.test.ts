import { describe, expect, it } from 'vitest'
import type { RedisBytes, RedisKeyRow } from '~/types/bindings'
import { byteLength, bytesId, bytesOf, bytesText, hexDump, parsedJson, sizeText, ttlText } from '~/utils/redisBytes'
import { foldersOf, treeRows } from '~/utils/redisTree'

const text = (t: string): RedisBytes => ({ kind: 'text', text: t })
const bin = (...bytes: number[]): RedisBytes => ({ kind: 'binary', bytes })
const key = (name: string, type = 'string', ttl: number | null = null): RedisKeyRow => ({ key: text(name), key_type: type, ttl_ms: ttl })

describe('redis bytes', () => {
  it('shows text as it is and binary with escapes, and never mixes the two up', () => {
    expect(bytesText(text('user:1042'))).toBe('user:1042')
    expect(bytesText(bin(0xff, 0x00, 0x41, 0x5c))).toBe('\\xff\\x00A\\x5c')
    // The same characters as text and as bytes are two different keys.
    expect(bytesId(text('A'))).not.toBe(bytesId(bin(0x41)))
    expect(bytesId(bin(1, 23))).not.toBe(bytesId(bin(12, 3)))
  })

  it('gives back the exact bytes', () => {
    expect([...bytesOf(text('سلام'))]).toEqual([0xd8, 0xb3, 0xd9, 0x84, 0xd8, 0xa7, 0xd9, 0x85])
    expect([...bytesOf(bin(255, 0))]).toEqual([255, 0])
    expect(byteLength(text('سلام'))).toBe(8)
    expect(byteLength(bin(1, 2, 3))).toBe(3)
  })

  it('dumps hex sixteen to a row with the readable characters beside', () => {
    const dump = hexDump(Uint8Array.from([0x52, 0x45, 0x44, 0x49, 0x53, 0x00, 0xff, 0x10, 0x41]))
    expect(dump).toBe('00000000  52 45 44 49 53 00 ff 10  41                       REDIS...A')
    expect(hexDump(new Uint8Array(40)).split('\n')).toHaveLength(3)
    expect(hexDump(new Uint8Array(100), 32).split('\n')).toHaveLength(2)
  })

  it('words time left and sizes', () => {
    expect(ttlText(83_640_000)).toBe('23h 14m')
    expect(ttlText(12 * 60_000)).toBe('12m')
    expect(ttlText(4 * 86_400_000 + 2 * 3_600_000)).toBe('4d 2h')
    expect(ttlText(45_000)).toBe('45s')
    expect(ttlText(3_600_000)).toBe('1h')
    expect(ttlText(400)).toBe('0s')
    expect(ttlText(83_640_000, 1)).toBe('23h')
    expect(sizeText(412)).toBe('412 B')
    expect(sizeText(2048)).toBe('2.0 KB')
    expect(sizeText(1_100_000)).toBe('1.0 MB')
  })

  it('treats only objects and arrays in text as JSON', () => {
    expect(parsedJson(text('{"beta":true}'))).toEqual({ beta: true })
    expect(parsedJson(text(' [1, 2] '))).toEqual([1, 2])
    expect(parsedJson(text('12'))).toBeUndefined()
    expect(parsedJson(text('"quoted"'))).toBeUndefined()
    expect(parsedJson(text('{not json'))).toBeUndefined()
    expect(parsedJson(bin(0x7b, 0x7d))).toBeUndefined()
  })
})

describe('redis key tree', () => {
  const keys = [
    key('user:1042:profile', 'hash', 83_640_000), key('user:1042:cart', 'list'), key('user:1042:roles', 'set'),
    key('user:1043:profile', 'hash'), key('user:1043:cart', 'list'),
    key('session:a'), key('session:b'), key('session:c'),
    key('leaderboard:weekly', 'zset'), key('maintenance'), key('config:pricing', 'json'),
  ]
  const names = (rows: ReturnType<typeof treeRows>) => rows.map(r => `${'  '.repeat(r.depth)}${r.kind === 'folder' ? `${r.name} (${r.count})` : r.name}`)

  it('groups by the separator, folders first, and shows a lone key in place of its folder', () => {
    expect(names(treeRows(keys, ':', new Set()))).toEqual([
      'session: (3)',
      'user: (5)',
      'config:pricing',
      'leaderboard:weekly',
      'maintenance',
    ])
  })

  it('opens folders level by level', () => {
    expect(names(treeRows(keys, ':', new Set(['user:', 'user:1042:'])))).toEqual([
      'session: (3)',
      'user: (5)',
      '  1042: (3)',
      '    cart',
      '    profile',
      '    roles',
      '  1043: (2)',
      'config:pricing',
      'leaderboard:weekly',
      'maintenance',
    ])
  })

  it('keeps binary keys whole, respects another separator, and has no folders without one', () => {
    const rows = treeRows([{ key: bin(0xff, 0x3a, 0x41), key_type: 'string', ttl_ms: null }, key('a/b/c'), key('a/b/d')], '/', new Set(['a/', 'a/b/']))
    expect(names(rows)).toEqual(['a/ (2)', '  b/ (2)', '    c', '    d', '\\xff:A'])
    expect(names(treeRows([key('a:b'), key('a:c')], '', new Set()))).toEqual(['a:b', 'a:c'])
  })

  it('keeps empty parts as part of the name', () => {
    const rows = treeRows([key('a::b'), key('a::c'), key('trailing:')], ':', new Set(['a:', 'a::']))
    expect(names(rows)).toEqual(['a: (2)', '  : (2)', '    b', '    c', 'trailing:'])
  })

  it('lists the folders that lead to a key', () => {
    expect(foldersOf('user:1042:profile', ':')).toEqual(['user:', 'user:1042:'])
    expect(foldersOf('maintenance', ':')).toEqual([])
    expect(foldersOf('a:b', '')).toEqual([])
  })
})

describe('redis editors (R3)', () => {
  it('reads an expiry the way a person types it', async () => {
    const { parseExpiry } = await import('~/utils/redisBytes')
    expect(parseExpiry('90s')).toBe(90_000)
    expect(parseExpiry('15m')).toBe(900_000)
    expect(parseExpiry(' 2H ')).toBe(7_200_000)
    expect(parseExpiry('7d')).toBe(604_800_000)
    expect(parseExpiry('1.5h')).toBe(5_400_000)
    const now = Date.parse('2026-11-16T10:00:00Z')
    expect(parseExpiry('2026-11-16T12:00:00Z', now)).toBe(7_200_000)
    // Not an expiry: nothing, zero, a bare number, words, a date that has passed.
    for (const bad of ['', '0s', '90', 'soon', '5 weeks', '-5m', '2020-01-01']) expect(parseExpiry(bad, now), bad).toBeNull()
  })

  it('says where JSON goes wrong, and nothing when it is valid', async () => {
    const { jsonProblem } = await import('~/utils/redisBytes')
    expect(jsonProblem('{"a":1}')).toBe('')
    expect(jsonProblem('[1, 2]')).toBe('')
    expect(jsonProblem('{"a":')).toContain('his isn\'t valid JSON yet.')
    expect(jsonProblem('{\n  "a": 1,\n  nope\n}')).toMatch(/^(Line 3: t|T)his isn't valid JSON yet\.$/)
  })
})
