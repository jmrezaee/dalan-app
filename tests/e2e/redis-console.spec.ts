import { expect, test } from '../support/fixtures'
import type { Page } from '@playwright/test'
import { openApp } from './harness'

// R4 of Redis (replica/redis/spec.md): the Console (S30, F16) and the Server view (S31).
// `local` is Open, `cache-prod` is Confirm writes (production), `sessions-prod` is Read-only.
async function openView(page: Page, name: string, view: 'Console' | 'Server') {
  const fake = await openApp(page, { scenario: 'team', unlocked: true })
  await page.getByRole('navigation', { name: 'Vault' }).getByRole('link', { name: /Databases/ }).click()
  await page.getByRole('row', { name: new RegExp(name) }).dblclick()
  const tab = page.getByTestId(`redis-tab-${name}`)
  await expect(tab.getByRole('tree', { name: 'Keys' })).toBeVisible()
  await tab.getByRole('tab', { name: view }).click()
  return { fake, tab, input: tab.getByRole('textbox', { name: 'Command' }), out: tab.getByRole('log', { name: 'Console output' }) }
}
const entries = (tab: ReturnType<Page['getByTestId']>) => tab.getByTestId('console-entry')
async function run(input: ReturnType<Page['getByRole']>, line: string) {
  await input.fill(line)
  await input.press('Enter')
}

test.describe('Console (S30, F16)', () => {
  test('a read runs at once and its reply is printed with types; the prompt names the database', async ({ page }) => {
    const { tab, input, out } = await openView(page, 'cache-prod', 'Console')
    await expect(input).toBeFocused()
    await run(input, 'TTL user:1042:profile')
    await expect(entries(tab).last()).toContainText('cache-prod:0> TTL user:1042:profile')
    await expect(entries(tab).last()).toContainText('(integer) 83640')
    await expect(entries(tab).last()).toContainText('· 0.4 ms')
    await run(input, 'HGETALL user:1043:profile')
    await expect(out).toContainText('1) "name"')
    await expect(out).toContainText('2) "Omid Karimi"')
    await run(input, 'GET nope')
    await expect(entries(tab).last()).toContainText('(nil)')
    // The server's own error is the reply.
    await run(input, 'GET user:1042:profile')
    await expect(entries(tab).last().locator('.text-danger')).toContainText('(error) WRONGTYPE')
    // A mistake in the line is explained and nothing is sent.
    await run(input, 'SET k "open')
    await expect(entries(tab).last()).toContainText('Not sent. A quote is never closed.')
  })

  test('Confirm writes: a write waits for Send, and Cancel sends nothing', async ({ page }) => {
    const { fake, tab, input } = await openView(page, 'cache-prod', 'Console')
    await run(input, 'EXPIRE maintenance 600')
    const e = entries(tab).last()
    await expect(e).toContainText('This changes data on production.')
    expect((await fake.redisKey('maintenance'))!.ttl_ms).toBe(720_000)
    await e.getByRole('button', { name: 'Cancel' }).click()
    await expect(e).toContainText('Cancelled. Nothing was sent.')
    expect((await fake.redisKey('maintenance'))!.ttl_ms).toBe(720_000)

    await run(input, 'EXPIRE maintenance 600')
    await entries(tab).last().getByRole('button', { name: 'Send' }).click()
    await expect(entries(tab).last()).toContainText('(integer) 1')
    expect((await fake.redisKey('maintenance'))!.ttl_ms).toBe(600_000)
  })

  test('Read-only: reads work, writes and dangerous commands are refused before they are sent', async ({ page }) => {
    const { fake, tab, input } = await openView(page, 'sessions-prod', 'Console')
    await run(input, 'GET maintenance')
    await expect(entries(tab).last()).toContainText('"Back at 14:30 UTC. Deploys are paused."')
    for (const line of ['DEL maintenance', 'SET maintenance gone', 'FLUSHDB', 'JSON.SET doc $ 1']) {
      await run(input, line)
      await expect(entries(tab).last()).toContainText('Not sent. sessions-prod is read-only in Dalan.')
    }
    expect(await fake.redisKey('maintenance')).not.toBeNull()
    expect((await fake.calls()).filter(c => c.startsWith('redis_write:'))).toHaveLength(0)
  })

  test('KEYS is not sent in any mode: the console offers SCAN, or "Run anyway"; MONITOR and SUBSCRIBE are always refused', async ({ page }) => {
    const { tab, input } = await openView(page, 'local', 'Console')
    await run(input, 'KEYS user:*')
    const e = entries(tab).last()
    await expect(e).toContainText('Not sent. KEYS walks every key at once and can stall a busy server.')
    await e.getByRole('button', { name: 'Use SCAN instead' }).click()
    await expect(input).toHaveValue('SCAN 0 MATCH user:* COUNT 500')
    await expect(e).toContainText('Cancelled. Nothing was sent.')
    await input.press('Enter')
    await expect(entries(tab).last()).toContainText('"user:1042:profile"')

    await run(input, 'KEYS user:1042:*')
    await entries(tab).last().getByRole('button', { name: 'Run anyway…' }).click()
    await expect(entries(tab).last()).toContainText('3) "user:1042:roles"')

    for (const line of ['MONITOR', 'SUBSCRIBE news', 'BLPOP q 0']) {
      await run(input, line)
      await expect(entries(tab).last()).toHaveAttribute('data-state', 'refused')
      await expect(entries(tab).last()).toContainText('Not sent.')
    }
  })

  test('a dangerous command needs the database\'s name, even in Open mode', async ({ page }) => {
    const { fake, tab, input } = await openView(page, 'local', 'Console')
    await run(input, 'FLUSHDB')
    const d = page.getByTestId('redis-danger')
    await expect(d.getByRole('heading', { name: 'Run FLUSHDB on local?' })).toBeVisible()
    await expect(d).toContainText('This removes all 1,237 keys in db0.')
    await expect(d.locator('[data-part="command"]')).toHaveText('FLUSHDB')
    await expect(d.getByRole('button', { name: 'Cancel' })).toBeFocused()
    await expect(d.getByRole('button', { name: 'Run FLUSHDB' })).toBeDisabled()
    await page.keyboard.press('Escape')
    await expect(entries(tab).last()).toContainText('Cancelled. Nothing was sent.')
    expect(await fake.redisKey('maintenance')).not.toBeNull()

    await run(input, 'FLUSHDB')
    await d.getByLabel('Type local to confirm').fill('local')
    await d.getByRole('button', { name: 'Run FLUSHDB' }).click()
    await expect(entries(tab).last()).toContainText('OK')
    expect(await fake.redisKey('maintenance')).toBeNull()
  })

  test('MULTI queues, EXEC takes the strongest class of its queue; SELECT moves the whole tab', async ({ page }) => {
    const { fake, tab, input } = await openView(page, 'cache-prod', 'Console')
    await run(input, 'MULTI')
    await expect(entries(tab).last()).toContainText('OK')
    await run(input, 'GET maintenance')
    await expect(entries(tab).last()).toContainText('QUEUED')
    await run(input, 'DEL flags:beta')
    await expect(entries(tab).last()).toContainText('QUEUED')
    await run(input, 'EXEC')
    // One write in the queue makes the whole EXEC a write: it asks first.
    await expect(entries(tab).last()).toContainText('This changes data on production.')
    expect(await fake.redisKey('flags:beta')).not.toBeNull()
    await entries(tab).last().getByRole('button', { name: 'Send' }).click()
    await expect(entries(tab).last()).toContainText('2) (integer) 1')
    expect(await fake.redisKey('flags:beta')).toBeNull()

    await run(input, 'SELECT 3')
    await expect(entries(tab).last()).toContainText('OK')
    await expect(tab.getByText('cache-prod:3>')).toBeVisible()
    await expect(tab.getByLabel('Logical database')).toContainText('db3')
  })

  test('completion, the syntax line, history with ↑/↓, and clearing', async ({ page }) => {
    const { fake, tab, input } = await openView(page, 'local', 'Console')
    await input.fill('hse')
    const popup = tab.getByTestId('console-complete')
    await expect(popup.getByRole('option')).toHaveText([/HSET\s*key field value \[field value \.\.\.\]/, /HSETNX\s*key field value/])
    await expect(popup).toContainText('Creates or modifies the value of a field in a hash.')
    await expect(popup).toContainText('Tab to complete')
    await input.press('Tab')
    await expect(input).toHaveValue('HSET ')
    await expect(tab.getByTestId('console-syntax')).toHaveText('HSET key field value [field value ...]')
    await input.fill('ge')
    await input.press('Escape')
    await expect(popup).toHaveCount(0)

    await run(input, 'PING')
    await run(input, 'GET maintenance')
    await run(input, 'AUTH hunter2')
    await input.fill('half typed')
    await input.press('ArrowUp')
    await expect(input).toHaveValue('AUTH hunter2') // this session's lines, including refused ones
    await input.press('ArrowUp')
    await expect(input).toHaveValue('GET maintenance')
    await input.press('ArrowUp')
    await expect(input).toHaveValue('PING')
    await input.press('ArrowDown')
    await input.press('ArrowDown')
    await input.press('ArrowDown')
    await expect(input).toHaveValue('half typed')

    // What Rust keeps never includes a line that carries a password.
    expect(await fake.redisHistory('r-local')).toEqual(['GET maintenance', 'PING'])
    const history = tab.getByRole('complementary', { name: 'History' })
    await history.getByRole('button', { name: 'PING' }).click()
    await expect(input).toHaveValue('PING')
    await expect(input).toBeFocused()

    await input.press(process.platform === 'darwin' ? 'Meta+l' : 'Control+l')
    await expect(entries(tab)).toHaveCount(0)
    await history.getByRole('button', { name: 'Clear' }).click()
    await expect(history).toContainText('Nothing yet.')
    expect(await fake.redisHistory('r-local')).toEqual([])
  })

  test('a long reply shows 1,000 lines and offers the rest; the output survives a visit to another view', async ({ page }) => {
    const { tab, input } = await openView(page, 'local', 'Console')
    // The fake's stand-in for a big reply (it classes as a write, and `local` is Open).
    await run(input, 'BIGREPLY 1500')
    const e = entries(tab).last()
    await expect(e).toContainText('1000) "row 1000"')
    await expect(e).not.toContainText('"row 1001"')
    await e.getByRole('button', { name: 'Show all 1,500 lines' }).click()
    await expect(e).toContainText('1500) "row 1500"')
    await tab.getByRole('tab', { name: 'Browser' }).click()
    await tab.getByRole('tab', { name: 'Console' }).click()
    await expect(entries(tab)).toHaveCount(1)
  })
})

test.describe('Server (S31)', () => {
  test('tiles, slow commands and keys by database come from the server', async ({ page }) => {
    const { tab } = await openView(page, 'cache-prod', 'Server')
    const view = tab.getByTestId('redis-server')
    for (const [label, value, note] of [['Version', '7.2.4', 'standalone'], ['Uptime', '41 days', 'since'], ['Memory', '1.8 GB', 'of 4.0 GB · 45%'], ['Clients', '212', '0 blocked'], ['Commands', '/s', 'now'], ['Hit rate', '96.2%', 'of 1,000 lookups']] as const) {
      const tile = view.locator('dl > div').filter({ hasText: label })
      await expect(tile).toContainText(value)
      await expect(tile).toContainText(note)
    }
    const slow = view.getByRole('table', { name: 'Slow commands' })
    await expect(slow.getByRole('row')).toHaveCount(4)
    await expect(slow.getByRole('row').nth(1)).toContainText('184 ms')
    await expect(slow.getByRole('row').nth(1)).toContainText('ZRANGEBYSCORE leaderboard:weekly -inf +inf')
    await expect(slow.getByRole('row').nth(1)).toContainText('10.0.1.14')
    await expect(view).toContainText('Dalan never turns on live command tracing.')
    const dbs = view.getByRole('table', { name: 'Keys by database' })
    await expect(dbs.getByRole('row').nth(1)).toContainText('db0')
    await expect(dbs.getByRole('row').nth(1)).toContainText('1,237')
    await expect(dbs.getByRole('row').nth(1)).toContainText('5h 12m')
    await expect(dbs.getByRole('row').nth(2)).toContainText('—')
  })

  test('"Clear log…" is a dangerous command: it asks for the name; Read-only has no such button', async ({ page }) => {
    const { tab } = await openView(page, 'cache-prod', 'Server')
    await tab.getByRole('button', { name: 'Clear log…' }).click()
    const d = page.getByTestId('redis-danger')
    await expect(d.getByRole('heading', { name: 'Clear the slow log on cache-prod?' })).toBeVisible()
    await expect(d.locator('[data-part="command"]')).toHaveText('SLOWLOG RESET')
    await d.getByLabel('Type cache-prod to confirm').fill('cache-prod')
    await d.getByRole('button', { name: 'Clear the slow log' }).click()
    await expect(tab.getByText('Nothing slow has been logged.')).toBeVisible()
  })

  test('Read-only hides "Clear log…"; refreshing is on a timer only while the view shows, and can be turned off', async ({ page }) => {
    const { fake, tab } = await openView(page, 'sessions-prod', 'Server')
    await expect(tab.getByRole('table', { name: 'Slow commands' })).toBeVisible()
    await expect(tab.getByRole('button', { name: 'Clear log…' })).toHaveCount(0)
    const first = await fake.redisInfoCalls()
    await tab.getByRole('button', { name: 'Refresh now' }).click()
    await expect.poll(() => fake.redisInfoCalls()).toBe(first + 1)
    const toggle = tab.getByRole('button', { name: 'Refresh every 5 s' })
    await expect(toggle).toHaveAttribute('aria-pressed', 'true')
    await toggle.click()
    await expect(toggle).toHaveAttribute('aria-pressed', 'false')
    // Another view: the Server view is gone, so nothing asks any more.
    await tab.getByRole('tab', { name: 'Browser' }).click()
    const after = await fake.redisInfoCalls()
    await page.waitForTimeout(1500)
    expect(await fake.redisInfoCalls()).toBe(after)
  })
})
