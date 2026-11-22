import { expect, test } from '../support/fixtures'
import type { Page } from '@playwright/test'
import { openApp } from './harness'

// R6 of Redis (replica/redis/spec.md): Cluster and Sentinel (S27, S31; F19). The routing itself (slots, MOVED,
// ASK) is Rust's and is tested there; here: the editor, the tab without a database picker, and the Nodes panel.
async function openDatabases(page: Page) {
  const fake = await openApp(page, { scenario: 'team', unlocked: true })
  await page.getByRole('navigation', { name: 'Vault' }).getByRole('link', { name: /Databases/ }).click()
  return fake
}
const sheet = (page: Page) => page.getByRole('dialog', { name: /database/i })

test.describe('editor (S27)', () => {
  test('a Sentinel database lists its sentinels and names its primary; the port follows the kind', async ({ page }) => {
    const fake = await openDatabases(page)
    await page.getByRole('button', { name: 'New database' }).click()
    await page.getByRole('radio', { name: /Redis/ }).click()
    await page.getByRole('button', { name: 'Continue' }).click()
    const s = sheet(page)
    await s.getByLabel('Name', { exact: true }).fill('jobs')
    await expect(s.getByLabel('Port')).toHaveValue('6379')
    await s.getByRole('radio', { name: 'Sentinel' }).click()
    await expect(s.getByText('List the sentinels. Dalan asks them where the primary is, and asks again if it moves.')).toBeVisible()
    await expect(s.getByLabel('Default port')).toHaveValue('26379')
    await s.getByLabel('Sentinels').fill('10.0.2.21:26379, 10.0.2.22')
    // The primary's name is required.
    await s.getByRole('button', { name: 'Save' }).click()
    await expect(s.getByRole('alert')).toHaveText('A Sentinel database needs the name of its primary.')
    await s.getByLabel('Primary\'s name').fill('mymaster')
    await s.getByRole('button', { name: 'Save' }).click()
    await expect(page.getByRole('row', { name: /jobs/ })).toContainText('Sentinel')
    const stored = await fake.redisDb('jobs')
    expect(stored).toMatchObject({ kind: 'sentinel', sentinel_master: 'mymaster', address: '10.0.2.21:26379, 10.0.2.22', port: 26379 })
  })

  test('a Cluster database takes the address of any node; a list is refused for it', async ({ page }) => {
    const fake = await openDatabases(page)
    await page.getByRole('button', { name: 'New database' }).click()
    await page.getByRole('radio', { name: /Redis/ }).click()
    await page.getByRole('button', { name: 'Continue' }).click()
    const s = sheet(page)
    await s.getByLabel('Name', { exact: true }).fill('events')
    await s.getByRole('radio', { name: 'Cluster' }).click()
    await expect(s.getByText('Give the address of any node.')).toBeVisible()
    await s.getByLabel('Address of any node').fill('10.0.2.11, 10.0.2.12')
    await s.getByRole('button', { name: 'Save' }).click()
    await expect(s.getByRole('alert')).toHaveText('The address is a host name or an IP address, without spaces or a path.')
    await s.getByLabel('Address of any node').fill('10.0.2.11')
    await s.getByLabel('Port').fill('7000')
    await s.getByRole('button', { name: 'Save' }).click()
    await expect(page.getByRole('row', { name: /^events\b|events / }).first()).toContainText('Cluster')
    expect(await fake.redisDb('events')).toMatchObject({ kind: 'cluster', port: 7000, sentinel_master: null })
  })
})

test.describe('tab (S28, S31; F19)', () => {
  test('a cluster tab has no database picker, browses its keys, and lists its nodes with slots and state', async ({ page }) => {
    await openDatabases(page)
    await page.getByRole('row', { name: /events-cluster/ }).dblclick()
    const tab = page.getByTestId('redis-tab-events-cluster')
    await expect(tab.getByRole('tree', { name: 'Keys' })).toBeVisible()
    await expect(tab.getByLabel('Logical database')).toHaveCount(0)
    await expect(tab.getByRole('treeitem', { name: /^user:/ })).toBeVisible()

    await tab.getByRole('tab', { name: 'Server' }).click()
    const nodes = tab.getByRole('table', { name: 'Nodes' })
    await expect(nodes.getByRole('row')).toHaveCount(7)
    await expect(nodes.getByRole('columnheader', { name: 'Slots' })).toBeVisible()
    await expect(nodes.getByRole('row').nth(1)).toHaveText(/primary\s*10\.0\.2\.11:7000\s*0–5460\s*ok/)
    await expect(nodes.getByRole('row').nth(2)).toHaveText(/replica\s*10\.0\.2\.14:7000\s*of 10\.0\.2\.11:7000\s*ok/)
    await expect(nodes.getByRole('row').nth(4)).toContainText('catching up')
    await expect(nodes.getByRole('row').nth(6)).toContainText('fail')
    await expect(tab.getByText('the tiles above are one node\'s; keys are the whole cluster\'s')).toBeVisible()
  })

  test('a Sentinel database asks the sentinels first, then opens like any other; its nodes are the set', async ({ page }) => {
    const fake = await openDatabases(page)
    await fake.redisManual()
    await page.getByRole('row', { name: /queue-sentinel/ }).dblclick()
    const tab = page.getByTestId('redis-tab-queue-sentinel')
    const conn = (await fake.redisConnIds()).at(-1)!
    await fake.redisEvent(conn, { type: 'plan', steps: [{ label: 'Sign in to bastion', detail: null }, { label: 'Ask the sentinels for mymaster', detail: null }, { label: 'Reach the primary', detail: null }, { label: 'Sign in to Redis', detail: null }] })
    await fake.redisEvent(conn, { type: 'step', index: 0, state: 'done', detail: null })
    await fake.redisEvent(conn, { type: 'step', index: 1, state: 'done', detail: '10.0.2.31:6379' })
    await fake.redisEvent(conn, { type: 'step', index: 2, state: 'active', detail: null })
    await expect(tab.getByText('Ask the sentinels for mymaster')).toBeVisible()
    await expect(tab.getByText('10.0.2.31:6379')).toBeVisible()
    await fake.redisEvent(conn, { type: 'ready', info: { version: '7.2.4', mode: 'standalone', replica: false, resp3: true, db_count: 16, databases: [{ index: 0, keys: 1237 }], selected: 0, latency_ms: 4 } })
    await expect(tab.getByLabel('Logical database')).toBeVisible()

    await tab.getByRole('tab', { name: 'Server' }).click()
    const nodes = tab.getByRole('table', { name: 'Nodes' })
    await expect(nodes.getByRole('columnheader', { name: 'Follows' })).toBeVisible()
    await expect(nodes.getByRole('row')).toHaveText([/Role/, /primary\s*10\.0\.2\.31:6379\s*mymaster\s*ok/, /replica/, /sentinel\s*10\.0\.2\.21:26379\s*watches the primary\s*ok/, /sentinel/])
    await expect(tab.getByText('as the sentinels see them')).toBeVisible()
  })

  test('a single server has no Nodes panel', async ({ page }) => {
    await openDatabases(page)
    await page.getByRole('row', { name: /local/ }).dblclick()
    const tab = page.getByTestId('redis-tab-local')
    await tab.getByRole('tab', { name: 'Server' }).click()
    await expect(tab.getByRole('table', { name: 'Keys by database' })).toBeVisible()
    await expect(tab.getByRole('table', { name: 'Nodes' })).toHaveCount(0)
  })
})
