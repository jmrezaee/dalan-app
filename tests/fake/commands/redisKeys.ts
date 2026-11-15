// The fake Redis keyspace (replica/redis/spec.md, R2): a small made-up database with every type, a binary key and
// value, a string too big to load at once, and enough keys that scanning takes several pages. No real names or data.
import type { RedisBytes } from '../../../app/types/bindings'

export type FakeValue =
  | { type: 'string', value: RedisBytes, totalBytes?: number, full?: RedisBytes }
  | { type: 'json', text: string }
  | { type: 'hash', fields: [string, string][] }
  | { type: 'list', items: string[] }
  | { type: 'set', members: string[] }
  | { type: 'zset', members: [string, number][] }
  | { type: 'stream', entries: { id: string, fields: [string, string][] }[], groups: { name: string, pending: number }[] }
  | { type: 'TSDB-TYPE' }

export interface FakeKey { key: RedisBytes, ttl_ms: number | null, value: FakeValue }

const t = (text: string): RedisBytes => ({ kind: 'text', text })
const MIN = 60_000
const H = 60 * MIN
const str = (name: string, value: string, ttl: number | null = null): FakeKey => ({ key: t(name), ttl_ms: ttl, value: { type: 'string', value: t(value) } })

/** db0. Built fresh for every page, so a test can remove keys without touching another test. */
export function keyspace(): FakeKey[] {
  const pad = (n: number) => String(n).padStart(4, '0')
  return [
    { key: t('user:1042:profile'), ttl_ms: 23 * H + 14 * MIN, value: { type: 'hash', fields: [['name', 'Sara Moradi'], ['email', 'sara@example.test'], ['plan', 'team'], ['locale', 'en-CA'], ['seats', '12'], ['created_at', '1730419200'], ['last_seen', '1794770112'], ['flags', '{"beta":true,"export":false}']] } },
    { key: t('user:1042:cart'), ttl_ms: 2 * H, value: { type: 'list', items: ['sku-8841', 'sku-1207', 'sku-5530'] } },
    { key: t('user:1042:roles'), ttl_ms: null, value: { type: 'set', members: ['billing', 'editor', 'owner'] } },
    { key: t('user:1043:profile'), ttl_ms: null, value: { type: 'hash', fields: [['name', 'Omid Karimi'], ['plan', 'solo']] } },
    { key: t('user:1043:cart'), ttl_ms: null, value: { type: 'list', items: ['sku-0001'] } },
    { key: t('user:1044:profile'), ttl_ms: null, value: { type: 'hash', fields: [['name', 'Lena Fischer']] } },
    { key: t('user:1044:cart'), ttl_ms: null, value: { type: 'list', items: [] } },
    { key: t('user:1044:roles'), ttl_ms: null, value: { type: 'set', members: ['viewer'] } },
    // 1,204 members: three pages of the sorted-set viewer (500, 500, 204).
    { key: t('leaderboard:weekly'), ttl_ms: 4 * 24 * H, value: { type: 'zset', members: Array.from({ length: 1204 }, (_, i): [string, number] => [`team:${pad(i)}`, 9812 - i * 7]) } },
    { key: t('events:orders'), ttl_ms: null, value: { type: 'stream', entries: [
      { id: '1794770112-0', fields: [['order', '5512'], ['status', 'paid'], ['total', '84.00']] },
      { id: '1794770098-0', fields: [['order', '5511'], ['status', 'paid'], ['total', '19.50']] },
      { id: '1794770031-1', fields: [['order', '5510'], ['status', 'refunded']] },
      { id: '1794770031-0', fields: [['order', '5509'], ['status', 'paid'], ['total', '42.10']] },
    ], groups: [{ name: 'billing', pending: 0 }, { name: 'emails', pending: 3 }] } },
    { key: t('config:pricing'), ttl_ms: null, value: { type: 'json', text: '{"currency":"CAD","tiers":[{"name":"solo","seats":1},{"name":"team","seats":12}],"trial_days":14}' } },
    str('maintenance', 'Back at 14:30 UTC. Deploys are paused.', 12 * MIN),
    str('flags:beta', '{"beta":true,"export":false}'),
    // A PNG header: bytes that are not text.
    { key: t('blob:thumb'), ttl_ms: null, value: { type: 'string', value: { kind: 'binary', bytes: [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d] } } },
    // A key whose name is not text: listed escaped, addressed by its bytes.
    { key: { kind: 'binary', bytes: [0xff, 0x00, 0x41] }, ttl_ms: null, value: { type: 'string', value: t('addressed by bytes') } },
    // 2 MB: opens as a preview until "Load all".
    { key: t('dump:big'), ttl_ms: null, value: { type: 'string', value: t('x'.repeat(200)), totalBytes: 2 * 1024 * 1024, full: t('x'.repeat(400)) } },
    { key: t('metrics:cpu'), ttl_ms: null, value: { type: 'TSDB-TYPE' } },
    ...Array.from({ length: 14 }, (_, i) => str(`feature:${String.fromCharCode(97 + i)}`, i % 2 ? 'on' : 'off')),
    ...Array.from({ length: 6 }, (_, i): FakeKey => ({ key: t(`queue:${i + 1}`), ttl_ms: null, value: { type: 'list', items: [`job-${i}`] } })),
    ...Array.from({ length: 1200 }, (_, i) => str(`session:${pad(i + 1)}`, `{"user":${1000 + i}}`, 30 * MIN)),
  ]
}

/** db3 holds two keys; every other logical database is empty. */
export const otherKeyspace = (index: number): FakeKey[] => (index === 3 ? [str('other:a', '1'), str('other:b', '2')] : [])

/** A Redis glob (`user:*`, `h?llo`, `[ab]*`) as a regular expression. */
export function globToRegExp(glob: string): RegExp {
  const body = glob.replace(/[.+^${}()|\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.')
  return new RegExp(`^${body}$`, 's')
}
