// Redis keys and values are bytes (replica/redis/spec.md). Rust sends each as text when it is readable text and as
// raw bytes otherwise; these helpers show either form and never change what a key is addressed by.
import type { RedisBytes } from '~/types/bindings'

const encoder = new TextEncoder()

/** The raw bytes of a value, whichever form it arrived in. */
export const bytesOf = (b: RedisBytes): Uint8Array => (b.kind === 'text' ? encoder.encode(b.text) : Uint8Array.from(b.bytes))

/** How to show it in one line: the text itself, or the bytes with `\xNN` escapes (printable ASCII stays readable). */
export function bytesText(b: RedisBytes): string {
  if (b.kind === 'text') return b.text
  let out = ''
  for (const n of b.bytes) out += n >= 0x20 && n < 0x7f && n !== 0x5c ? String.fromCharCode(n) : `\\x${n.toString(16).padStart(2, '0')}`
  return out
}

/** A string that identifies the bytes, for list keys and "is this the selected key". Text and binary never collide. */
export const bytesId = (b: RedisBytes): string => (b.kind === 'text' ? `t:${b.text}` : `b:${b.bytes.join(',')}`)

export const byteLength = (b: RedisBytes): number => (b.kind === 'text' ? encoder.encode(b.text).length : b.bytes.length)

/** A classic hex dump: offset, 16 bytes in hex, then the printable characters. */
export function hexDump(bytes: Uint8Array, limit = 64 * 1024): string {
  const lines: string[] = []
  const end = Math.min(bytes.length, limit)
  for (let at = 0; at < end; at += 16) {
    const row = bytes.subarray(at, Math.min(at + 16, end))
    const hex = Array.from(row, n => n.toString(16).padStart(2, '0'))
    const left = hex.slice(0, 8).join(' ')
    const right = hex.slice(8).join(' ')
    const text = Array.from(row, n => (n >= 0x20 && n < 0x7f ? String.fromCharCode(n) : '.')).join('')
    lines.push(`${at.toString(16).padStart(8, '0')}  ${left.padEnd(23)}  ${right.padEnd(23)}  ${text}`)
  }
  return lines.join('\n')
}

/** "23h 14m", "12m", "4d 2h", "45s". `null` (no expiry) is the caller's to word. */
export function ttlText(ms: number, parts = 2): string {
  const s = Math.max(0, Math.round(ms / 1000))
  const units: [number, string][] = [[Math.floor(s / 86400), 'd'], [Math.floor((s % 86400) / 3600), 'h'], [Math.floor((s % 3600) / 60), 'm'], [s % 60, 's']]
  const first = units.findIndex(([n]) => n > 0)
  if (first < 0) return '0s'
  return units.slice(first, first + parts).filter(([n]) => n > 0).map(([n, u]) => `${n}${u}`).join(' ')
}

/** What the expiry control accepts: `90s`, `15m`, `2h`, `7d`, or a date in the future. Milliseconds from `now`, or `null`. */
export function parseExpiry(text: string, now = Date.now()): number | null {
  const t = text.trim().toLowerCase()
  const m = /^(\d+(?:\.\d+)?)\s*(s|m|h|d)$/.exec(t)
  if (m) {
    const ms = Math.round(Number(m[1]) * { s: 1000, m: 60_000, h: 3_600_000, d: 86_400_000 }[m[2] as 's' | 'm' | 'h' | 'd'])
    return ms > 0 ? ms : null
  }
  // A bare number is not a date ("90" would be the year 90).
  if (!t || /^\d+$/.test(t)) return null
  const at = Date.parse(text.trim())
  return Number.isFinite(at) && at > now ? at - now : null
}

/** Text as the bytes Rust expects for a key, a field or a value the person typed. */
export const textBytes = (text: string): RedisBytes => ({ kind: 'text', text })

/** "Line 6" for a JSON mistake, when the browser says where it is; `''` when the text is valid. */
export function jsonProblem(text: string): string {
  try {
    JSON.parse(text)
    return ''
  }
  catch (e) {
    const at = /position (\d+)/.exec(e instanceof Error ? e.message : '')
    const line = at ? text.slice(0, Number(at[1])).split('\n').length : 0
    return `${line ? `Line ${line}: t` : 'T'}his isn't valid JSON yet.`
  }
}

/** "412 B", "2.0 KB", "1.1 MB". */
export function sizeText(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1024 ** 3) return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`
}

export const count = (n: number) => n.toLocaleString('en-US')

/** The value as parsed JSON when it is an object or an array in text; `undefined` otherwise (a bare `12` is just a string). */
export function parsedJson(b: RedisBytes): unknown {
  if (b.kind !== 'text') return undefined
  const t = b.text.trim()
  if (!(t.startsWith('{') || t.startsWith('['))) return undefined
  try {
    return JSON.parse(t)
  }
  catch {
    return undefined
  }
}

/** The short type word on a chip. */
export const TYPE_LABEL: Record<string, string> = { string: 'str', hash: 'hash', list: 'list', set: 'set', zset: 'zset', stream: 'stream', json: 'json' }
/** What a type's items are called, for "8 fields" and "This hash has no fields." */
export const TYPE_UNIT: Record<string, [one: string, many: string]> = {
  hash: ['field', 'fields'], list: ['item', 'items'], set: ['member', 'members'], zset: ['member', 'members'], stream: ['entry', 'entries'],
}

export interface JsonToken { text: string, kind: 'key' | 'string' | 'number' | 'word' | 'plain' }

/** Pretty-printed JSON as coloured pieces (keys, strings, numbers, true/false/null), for the JSON view. No HTML is built. */
export function jsonTokens(value: unknown): JsonToken[] {
  const text = JSON.stringify(value, null, 2) ?? 'null'
  const out: JsonToken[] = []
  // A string (maybe followed by a colon, which makes it a key), a number, or a bare word.
  const re = /("(?:[^"\\]|\\.)*")(\s*:)?|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|\b(true|false|null)\b/g
  let last = 0
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (m.index > last) out.push({ text: text.slice(last, m.index), kind: 'plain' })
    if (m[1] !== undefined) {
      out.push({ text: m[1], kind: m[2] ? 'key' : 'string' })
      if (m[2]) out.push({ text: m[2], kind: 'plain' })
    }
    else if (m[3] !== undefined) out.push({ text: m[3], kind: 'number' })
    else out.push({ text: m[4]!, kind: 'word' })
    last = m.index + m[0].length
  }
  if (last < text.length) out.push({ text: text.slice(last), kind: 'plain' })
  return out
}
