import { expect, test } from '../support/fixtures'
import type { Page } from '@playwright/test'
import { openApp } from './harness'

// R5 of Redis (replica/redis/spec.md): Pub/Sub (S32, F17) and bulk actions (S33, F18).
async function openDb(page: Page, name: string, view?: 'Pub/Sub') {
  const fake = await openApp(page, { scenario: 'team', unlocked: true })
  await page.getByRole('navigation', { name: 'Vault' }).getByRole('link', { name: /Databases/ }).click()
  await page.getByRole('row', { name: new RegExp(name) }).dblclick()
  const tab = page.getByTestId(`redis-tab-${name}`)
  await expect(tab.getByRole('tree', { name: 'Keys' })).toBeVisible()
  if (view) await tab.getByRole('tab', { name: view }).click()
  return { fake, tab }
}
async function listen(tab: ReturnType<Page['getByTestId']>, name: string) {
  await tab.getByLabel('Channel or pattern').fill(name)
  await tab.getByRole('button', { name: 'Listen', exact: true }).click()
}
const rows = (tab: ReturnType<Page['getByTestId']>) => tab.getByRole('table', { name: 'Messages' }).getByRole('row')

test.describe('Pub/Sub (S32, F17)', () => {
  test('listening to a channel and a pattern shows messages newest first, with a count per name', async ({ page }) => {
    const { fake, tab } = await openDb(page, 'cache-prod', 'Pub/Sub')
    await expect(tab.getByText('Not listening to anything yet.')).toBeVisible()
    await expect(tab.getByText('Listen to a channel to see its messages here.')).toBeVisible()
    await listen(tab, 'deploys')
    await listen(tab, 'orders.*')
    const list = tab.getByRole('region', { name: 'Listening to' }).getByRole('listitem')
    await expect(list).toHaveCount(2)
    await expect(list.nth(0)).toContainText('deploys')
    await expect(list.nth(0)).toContainText('channel')
    await expect(list.nth(1)).toContainText('orders.*')
    await expect(list.nth(1)).toContainText('pattern')
    expect(await fake.redisSubNames()).toEqual(['deploys', 'orders.*'])

    await fake.redisPublish('deploys', 'api v2.41.0 started')
    await fake.redisPublish('orders.paid', '{"order":5512,"total":84.00}')
    await fake.redisPublish('nobody.listens', 'lost')
    await expect(rows(tab)).toHaveCount(3) // the header and two messages
    await expect(rows(tab).nth(1)).toContainText('orders.paid')
    await expect(rows(tab).nth(1)).toContainText('{"order":5512,"total":84.00}')
    await expect(rows(tab).nth(2)).toContainText('api v2.41.0 started')
    await expect(rows(tab).nth(1).getByRole('cell').first()).toHaveText(/^\d\d:\d\d:\d\d\.\d{3}$/)
    await expect(list.nth(0)).toContainText('1')
    await expect(list.nth(1)).toContainText('1')
    await expect(tab.getByText('2 received · keeping the last 5,000')).toBeVisible()

    // The filter narrows what is kept; stopping one name stops its messages.
    await tab.getByLabel('Filter messages').fill('5512')
    await expect(rows(tab)).toHaveCount(2)
    await tab.getByLabel('Filter messages').fill('')
    await tab.getByRole('button', { name: 'Stop listening to deploys' }).click()
    await expect(list).toHaveCount(1)
    await fake.redisPublish('deploys', 'unheard')
    await fake.redisPublish('orders.refunded', '{"order":5510}')
    await expect(rows(tab)).toHaveCount(4)
    await expect(tab.getByText('unheard')).toHaveCount(0)
  })

  test('Pause keeps receiving and says how many wait; Resume shows them; Clear empties the list', async ({ page }) => {
    const { fake, tab } = await openDb(page, 'local', 'Pub/Sub')
    await listen(tab, 'deploys')
    await fake.redisPublish('deploys', 'one')
    await expect(rows(tab)).toHaveCount(2)
    const pause = tab.getByRole('button', { name: 'Pause' })
    await pause.click()
    await fake.redisPublish('deploys', 'while paused', 3)
    await expect(tab.getByText('Paused · 3 waiting')).toBeVisible()
    await expect(rows(tab)).toHaveCount(2)
    await tab.getByRole('button', { name: 'Resume' }).click()
    await expect(rows(tab)).toHaveCount(5)
    await expect(rows(tab).nth(1)).toContainText('while paused 3')
    await tab.getByRole('button', { name: 'Clear' }).click()
    await expect(tab.getByText('Nothing has arrived yet.')).toBeVisible()
    await expect(tab.getByText('0 received')).toBeVisible()
  })

  test('a flood does not freeze the tab: 20,000 messages arrive, the last 5,000 are kept, the rest are counted', async ({ page }) => {
    const { fake, tab } = await openDb(page, 'local', 'Pub/Sub')
    await listen(tab, 'flood')
    await fake.redisPublish('flood', 'm', 20_000)
    await expect(tab.getByText('20,000 received · keeping the last 5,000 · 15,000 dropped')).toBeVisible()
    // Only the rows near the viewport are drawn.
    expect(await rows(tab).count()).toBeLessThan(80)
    // Still responsive: another view opens at once and comes back with the messages.
    await tab.getByRole('tab', { name: 'Browser' }).click()
    await expect(tab.getByRole('tree', { name: 'Keys' })).toBeVisible()
    await tab.getByRole('tab', { name: 'Pub/Sub' }).click()
    await expect(tab.getByText('20,000 received')).toBeVisible()
  })

  test('publishing is a write: reviewed in Confirm mode, sent at once in Open mode, hidden in Read-only', async ({ page }) => {
    const { fake, tab } = await openDb(page, 'cache-prod', 'Pub/Sub')
    await listen(tab, 'deploys')
    await tab.getByLabel('Channel to publish to').fill('deploys')
    await tab.getByLabel('Message', { exact: true }).fill('api v2.42.0 started')
    await tab.getByRole('button', { name: 'Review and publish' }).click()
    const d = page.getByTestId('redis-confirm')
    await expect(d.getByRole('heading', { name: 'Publish this message?' })).toBeVisible()
    await expect(d.locator('[data-part="command"]')).toHaveText('PUBLISH deploys "api v2.42.0 started"')
    await expect(d.getByRole('button', { name: 'Cancel' })).toBeFocused()
    expect((await fake.calls()).filter(c => c === 'redis_write:publish')).toHaveLength(0)
    await d.getByRole('button', { name: 'Publish' }).click()
    await expect(rows(tab).nth(1)).toContainText('api v2.42.0 started')
    await expect(page.getByText('Published.')).toBeVisible()
    await expect(tab.getByLabel('Message', { exact: true })).toHaveValue('')
    expect((await fake.calls()).filter(c => c === 'redis_write:publish')).toHaveLength(1)
  })

  test('Read-only can listen but not publish; an old server says why it cannot listen; closing the tab stops listening', async ({ page }) => {
    const { fake, tab } = await openDb(page, 'sessions-prod', 'Pub/Sub')
    await expect(tab.getByRole('button', { name: /publish/i })).toHaveCount(0)
    await listen(tab, 'old-server')
    await expect(tab.getByRole('alert')).toHaveText('Listening needs Redis 6 or newer (this server speaks the older protocol).')
    await listen(tab, 'deploys')
    await fake.redisPublish('deploys', 'heard')
    await expect(rows(tab).nth(1)).toContainText('heard')
    await page.getByRole('button', { name: 'Close sessions-prod' }).click()
    await expect.poll(() => fake.redisSubNames()).toEqual([])
  })

  test('after the connection drops and comes back, the tab listens again by itself', async ({ page }) => {
    const { fake, tab } = await openDb(page, 'local', 'Pub/Sub')
    await listen(tab, 'deploys')
    const conn = (await fake.redisOpenConnIds()).at(-1)!
    await fake.redisEvent(conn, { type: 'dropped', attempt: 1, of: 5, retry_in_secs: 1 })
    await fake.redisEvent(conn, { type: 'ready', info: { version: '7.2.4', mode: 'standalone', replica: false, resp3: true, db_count: 16, databases: [{ index: 0, keys: 1237 }], selected: 0, latency_ms: 3 } })
    await tab.getByRole('tab', { name: 'Pub/Sub' }).click()
    await expect.poll(async () => (await fake.calls()).filter(c => c === 'redis_sub:c:deploys').length).toBe(2)
    await fake.redisPublish('deploys', 'back')
    await expect(rows(tab).nth(1)).toContainText('back')
  })
})

test.describe('Bulk actions (S33, F18)', () => {
  const dialog = (page: Page) => page.getByTestId('redis-bulk')

  test('preview shows the count and a sample; the action stays disabled until the database\'s name is typed; then it runs', async ({ page }) => {
    const { fake, tab } = await openDb(page, 'cache-prod')
    await tab.getByRole('button', { name: 'Bulk…' }).click()
    const d = dialog(page)
    await expect(d.getByRole('heading', { name: 'Actions on matching keys' })).toBeVisible()
    await expect(d.getByRole('button', { name: 'Cancel' })).toBeFocused()
    const go = d.getByRole('button', { name: /^Delete/ })
    await expect(go).toBeDisabled()
    await d.getByLabel('Key pattern').fill('session:*')
    await d.getByRole('button', { name: 'Preview' }).click()
    const preview = d.getByTestId('bulk-preview')
    await expect(preview).toContainText('1,200 keys match')
    await expect(preview).toContainText('whole database scanned')
    await expect(preview).toContainText('session:0001 · session:0002')
    await expect(preview).toContainText('… 1,180 more')
    await expect(d.getByRole('button', { name: 'Delete 1,200 keys' })).toBeDisabled()
    const name = d.getByLabel('Type cache-prod to confirm')
    await name.fill('cache-pro')
    await expect(d.getByRole('button', { name: 'Delete 1,200 keys' })).toBeDisabled()
    expect(await fake.redisKeyCount()).toBe(1237)

    await name.fill('cache-prod')
    await d.getByRole('button', { name: 'Delete 1,200 keys' }).click()
    await expect(d.getByTestId('bulk-result')).toHaveText(/Deleted 1,200 keys\./)
    expect(await fake.redisKeyCount()).toBe(37)
    expect(await fake.redisKey('maintenance')).not.toBeNull()
    await d.locator('[data-cancel]').click()
    // The tree was scanned again: the sessions are gone.
    await expect(tab.getByRole('treeitem', { name: /^session:/ })).toHaveCount(0)
    await expect(tab.getByText('All 37 keys scanned')).toBeVisible()
  })

  test('set and remove an expiry on a pattern; changing the pattern clears the preview; nothing matching says so', async ({ page }) => {
    const { fake, tab } = await openDb(page, 'local')
    await tab.getByRole('button', { name: 'Bulk…' }).click()
    const d = dialog(page)
    await d.getByLabel('Key pattern').fill('feature:*')
    await d.getByRole('button', { name: 'Preview' }).click()
    await expect(d.getByTestId('bulk-preview')).toContainText('14 keys match')
    await d.getByRole('radio', { name: 'Set expiry' }).click()
    await d.getByLabel('Type local to confirm').fill('local')
    const go = d.getByRole('button', { name: 'Set expiry on 14 keys' })
    await expect(go).toBeDisabled() // no expiry typed yet
    await d.getByLabel('Expires in').fill('soon')
    await expect(go).toBeDisabled()
    await d.getByLabel('Expires in').fill('2h')
    await go.click()
    await expect(d.getByTestId('bulk-result')).toHaveText(/Set the expiry on 14 keys\./)
    expect((await fake.redisKey('feature:a'))!.ttl_ms).toBe(7_200_000)

    // Another pattern is another set of keys.
    await d.getByLabel('Key pattern').fill('feature:a')
    await expect(d.getByTestId('bulk-result')).toHaveCount(0)
    await expect(d.getByTestId('bulk-preview')).toHaveCount(0)
    await d.getByRole('button', { name: 'Preview' }).click()
    await expect(d.getByTestId('bulk-preview')).toContainText('1 key matches')
    await d.getByRole('radio', { name: 'Remove expiry' }).click()
    await d.getByLabel('Type local to confirm').fill('local')
    await d.getByRole('button', { name: 'Remove expiry from 1 key' }).click()
    await expect(d.getByTestId('bulk-result')).toHaveText(/Removed the expiry from 1 key\./)
    expect((await fake.redisKey('feature:a'))!.ttl_ms).toBeNull()

    await d.getByLabel('Key pattern').fill('nothing:*')
    await d.getByRole('button', { name: 'Preview' }).click()
    await expect(d.getByTestId('bulk-preview')).toContainText('No keys match nothing:*.')
    await expect(d.getByLabel('Type local to confirm')).toHaveCount(0)
  })

  test('Stop during a run keeps what is done and leaves the rest alone', async ({ page }) => {
    const { fake, tab } = await openDb(page, 'local')
    await fake.redisBulkSlow()
    await tab.getByRole('button', { name: 'Bulk…' }).click()
    const d = dialog(page)
    await d.getByLabel('Key pattern').fill('session:*')
    await d.getByRole('button', { name: 'Preview' }).click()
    await expect(d.getByTestId('bulk-preview')).toContainText('1,200 keys match')
    await d.getByLabel('Type local to confirm').fill('local')
    await d.getByRole('button', { name: 'Delete 1,200 keys' }).click()
    await expect(d.getByText(/Deleting 500 of 1,200 · in small batches/)).toBeVisible()
    await d.getByRole('button', { name: 'Stop' }).click()
    await expect(d.getByTestId('bulk-result')).toHaveText(/Stopped\. (500|1,000) of 1,200 done; the rest were left alone\./)
    const left = await fake.redisKeyCount()
    expect([737, 237]).toContain(left)
  })

  test('Read-only has no Bulk button, and the gate refuses a run that is asked for anyway', async ({ page }) => {
    const { fake, tab } = await openDb(page, 'sessions-prod')
    await expect(tab.getByRole('button', { name: 'Bulk…' })).toHaveCount(0)
    const conn = (await fake.redisOpenConnIds()).at(-1)!
    const code = await page.evaluate(async (connId) => {
      const invoke = (window as any).__TAURI_INTERNALS__.invoke
      return invoke('redis_bulk_run', { connId, previewId: 'made-up', action: { action: 'delete' }, typedName: 'sessions-prod', progress: null }).then(() => 'ran', (e: { code: string }) => e.code)
    }, conn)
    expect(code).toBe('redis_read_only_mode')
    expect(await fake.redisKeyCount()).toBe(1237)
  })
})
