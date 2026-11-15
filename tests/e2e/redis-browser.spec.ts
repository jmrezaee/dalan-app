import { expect, test } from '../support/fixtures'
import type { Page } from '@playwright/test'
import { openApp } from './harness'

// R2 of Redis (replica/redis/spec.md): the Browser's read side (S28, S29; F14). Find a key by pattern, type and tree,
// and read its value in pages. These run on a Read-only database (sessions-prod), so nothing here can change data
// and the viewers show values as plain text; the editors are in redis-write.spec.ts. The fake holds a small made-up keyspace.
async function openBrowser(page: Page, scenario: 'team' = 'team') {
  const fake = await openApp(page, { scenario, unlocked: true })
  await page.getByRole('navigation', { name: 'Vault' }).getByRole('link', { name: /Databases/ }).click()
  await page.getByRole('row', { name: /sessions-prod/ }).dblclick()
  const tab = page.getByTestId('redis-tab-sessions-prod')
  await expect(tab.getByRole('tree', { name: 'Keys' })).toBeVisible()
  return { fake, tab, tree: tab.getByRole('tree', { name: 'Keys' }), value: tab.getByTestId('redis-value') }
}
const item = (tab: ReturnType<Page['getByTestId']>, name: string | RegExp) => tab.getByRole('treeitem', { name })

test.describe('key tree (S28, F14)', () => {
  test('keys are grouped by the separator, with counts; a folder opens to its keys with type and time left', async ({ page }) => {
    const { tab } = await openBrowser(page)
    // The first page of the scan: every top-level folder, each with how many keys it holds so far.
    await expect(item(tab, /^user:/)).toContainText('8')
    await expect(item(tab, /^feature:/)).toContainText('14')
    await expect(item(tab, /^queue:/)).toContainText('6')
    // A folder with one key is shown as that key.
    await expect(item(tab, /leaderboard:weekly/)).toContainText('zset')
    await expect(item(tab, /maintenance/)).toContainText('str')
    await expect(item(tab, /maintenance/)).toContainText('12m')
    // A key whose name is not text is listed with escapes.
    await expect(item(tab, /\\xff\\x00A/)).toBeVisible()

    await item(tab, /^user:/).click()
    await expect(item(tab, /^user:/)).toHaveAttribute('aria-expanded', 'true')
    await item(tab, /^1042:/).click()
    await expect(item(tab, /profile/).first()).toContainText('hash')
    await expect(item(tab, /profile/).first()).toContainText('23h')
    await expect(item(tab, /roles/).first()).toContainText('no expiry')
    await expect(page.getByRole('heading', { name: 'Pick a key to see its value.' })).toBeVisible()
  })

  test('the footer says how much was scanned; "Scan more" continues until the whole database was walked', async ({ page }) => {
    const { tab } = await openBrowser(page)
    const total = 1237
    await expect(tab.getByText(`Scanned 500 of ${total.toLocaleString('en-US')} keys`)).toBeVisible()
    await expect(item(tab, /^session:/)).toContainText('463') // 500 looked at, 37 of them were not sessions
    await tab.getByRole('button', { name: 'Scan more' }).click()
    await expect(tab.getByText('Scanned 1,000 of 1,237 keys')).toBeVisible()
    await tab.getByRole('button', { name: 'Scan more' }).click()
    await expect(tab.getByText('All 1,237 keys scanned')).toBeVisible()
    await expect(item(tab, /^session:/)).toContainText('1,200')
    await expect(tab.getByRole('button', { name: 'Scan more' })).toHaveCount(0)
  })

  test('a pattern and a type narrow the list on the server (SCAN, never KEYS); no match says so', async ({ page }) => {
    const { fake, tab } = await openBrowser(page)
    await tab.getByLabel('Key pattern').fill('user:*')
    await tab.getByLabel('Key pattern').press('Enter')
    await expect(item(tab, /^user:/)).toContainText('8')
    await expect(item(tab, /^feature:/)).toHaveCount(0)

    await tab.getByRole('combobox', { name: 'Filter by type' }).click()
    await page.getByRole('option', { name: 'Hash' }).click()
    await expect(item(tab, /^user:/)).toContainText('3')

    await tab.getByLabel('Key pattern').fill('nothing:*')
    await tab.getByLabel('Key pattern').press('Enter')
    await expect(tab.getByText('No keys match nothing:* in what\'s been scanned so far. Scan more?')).toBeVisible()
    // Every listing went through redis_scan; the app has no way to ask for KEYS.
    const redisCalls = (await fake.calls()).filter(c => c.startsWith('redis_'))
    expect(redisCalls.filter(c => c === 'redis_scan').length).toBeGreaterThanOrEqual(4)
    expect(redisCalls.filter(c => /keys/i.test(c))).toEqual([])
  })

  test('the tree works from the keyboard: arrows move, Right and Left open and close, Enter opens a key', async ({ page }) => {
    const { tab, tree, value } = await openBrowser(page)
    await tree.focus()
    // Folders come first, by name: feature:, queue:, session:, user:
    await page.keyboard.press('End')
    await page.keyboard.press('Home')
    for (let i = 0; i < 3; i++) await page.keyboard.press('ArrowDown')
    await page.keyboard.press('ArrowRight') // open user:
    await expect(item(tab, /^user:/)).toHaveAttribute('aria-expanded', 'true')
    await page.keyboard.press('ArrowRight') // into it: 1042:
    await page.keyboard.press('ArrowRight') // open 1042:
    await page.keyboard.press('ArrowDown') // cart
    await page.keyboard.press('Enter')
    await expect(value.getByRole('heading', { name: 'user:1042:cart' })).toBeVisible()
    await page.keyboard.press('ArrowLeft') // back to the folder 1042:
    await page.keyboard.press('ArrowLeft') // close it
    await expect(item(tab, /^1042:/)).toHaveAttribute('aria-expanded', 'false')
  })
})

/** Opens a key by typing its exact name as the pattern, then clicking the one row. */
async function openKey(page: Page, tab: ReturnType<Page['getByTestId']>, name: string, row: string | RegExp = name) {
  await tab.getByLabel('Key pattern').fill(name)
  await tab.getByLabel('Key pattern').press('Enter')
  await item(tab, row).first().click()
}

test.describe('values by type (S28, S29)', () => {
  test('hash: the header says what the key is; fields load into a table and filter', async ({ page }) => {
    const { tab, value } = await openBrowser(page)
    await openKey(page, tab, 'user:1042:profile')
    await expect(value.getByRole('heading', { name: 'user:1042:profile' })).toBeVisible()
    await expect(value).toContainText('Expires in 23h 14m')
    await expect(value).toContainText('8 fields · 412 B')
    const table = value.getByRole('table', { name: 'Fields' })
    await expect(table.getByRole('row')).toHaveCount(9) // the header and eight fields
    await expect(table.getByRole('row', { name: /^email/ })).toContainText('sara@example.test')
    await value.getByLabel('Filter fields').fill('seat')
    await expect(table.getByRole('row')).toHaveCount(2)
    await expect(value.getByText('1 of 8 loaded')).toBeVisible()
    await expect(value.getByText('All 8 fields loaded')).toBeVisible()
    // The read side has nothing to change data with.
    await expect(value.getByRole('button', { name: /Add|Edit|Remove|Rename|Delete|Review/ })).toHaveCount(0)
  })

  test('string: text with its size; a string that is JSON opens as JSON and can be read as text or hex', async ({ page }) => {
    const { tab, value } = await openBrowser(page)
    await openKey(page, tab, 'maintenance')
    await expect(value.getByLabel('Value as text')).toHaveText('Back at 14:30 UTC. Deploys are paused.')
    await expect(value).toContainText('38 bytes · text')
    await expect(value).toContainText('Expires in 12m')
    await expect(value.getByRole('radio', { name: 'JSON' })).toHaveCount(0)

    await openKey(page, tab, 'flags:beta')
    await expect(value.getByRole('radio', { name: 'JSON' })).toHaveAttribute('aria-checked', 'true')
    await expect(value.getByLabel('Value as JSON')).toContainText('"beta": true')
    await value.getByRole('radio', { name: 'Hex' }).click()
    await expect(value.getByLabel('Value as hex')).toContainText('7b 22 62 65 74 61')
    await value.getByRole('radio', { name: 'Text' }).click()
    await expect(value.getByLabel('Value as text')).toHaveText('{"beta":true,"export":false}')
  })

  test('binary: a binary value opens as hex, read-only; a binary key is opened by its bytes', async ({ page }) => {
    const { fake, tab, value } = await openBrowser(page)
    await openKey(page, tab, 'blob:thumb')
    await expect(value.getByLabel('Value as hex')).toContainText('89 50 4e 47 0d 0a 1a 0a')
    await expect(value.getByLabel('Value as hex')).toContainText('.PNG')
    await expect(value).toContainText('Binary values open as hex and are read-only.')
    await expect(value).toContainText('12 bytes · binary')
    await expect(value.getByRole('radio', { name: 'Text' })).toHaveCount(0)

    await tab.getByLabel('Key pattern').fill('')
    await tab.getByLabel('Key pattern').press('Enter')
    await item(tab, /\\xff\\x00A/).click()
    await expect(value.getByRole('heading', { name: '\\xff\\x00A' })).toBeVisible()
    await expect(value.getByLabel('Value as text')).toHaveText('addressed by bytes')
    // The key went to the backend as its bytes, not as the escaped text.
    const asked = await page.evaluate(() => (window as any).__fake.state.answers as string[])
    expect(asked.some(a => a.startsWith('redis_key_info') && a.includes('"bytes":[255,0,65]'))).toBe(true)
    expect(await fake.unhandled()).toEqual([])
  })

  test('a string over 1 MB opens as a preview; "Load all" brings the rest', async ({ page }) => {
    const { fake, tab, value } = await openBrowser(page)
    await openKey(page, tab, 'dump:big')
    await expect(value).toContainText('Showing the first 200 B of 2.0 MB.')
    await expect(value.getByLabel('Value as text')).toHaveText('x'.repeat(200))
    await value.getByRole('button', { name: 'Load all' }).click()
    await expect(value.getByLabel('Value as text')).toHaveText('x'.repeat(400))
    await expect(value.getByRole('button', { name: 'Load all' })).toHaveCount(0)
    expect(await fake.calls('redis_value_page')).toHaveLength(2)
  })

  test('list, set and stream: each in its own table', async ({ page }) => {
    const { tab, value } = await openBrowser(page)
    await openKey(page, tab, 'user:1042:cart')
    const list = value.getByRole('table', { name: 'Items' })
    await expect(list.getByRole('row').nth(1)).toHaveText(/0\s*sku-8841/)
    await expect(list.getByRole('row').nth(3)).toHaveText(/2\s*sku-5530/)
    await expect(value).toContainText('3 items')

    await openKey(page, tab, 'user:1042:roles')
    await expect(value.getByRole('table', { name: 'Members' }).getByRole('row')).toHaveText(['Member', 'billing', 'editor', 'owner'])

    await openKey(page, tab, 'events:orders')
    const stream = value.getByRole('table', { name: 'Entries' })
    await expect(stream.getByRole('row').nth(1)).toContainText('1794770112-0')
    await expect(stream.getByRole('row').nth(1)).toContainText('order=5512 status=paid total=84.00')
    await expect(value).toContainText('Groups: billing (0 waiting), emails (3 waiting).')
    await expect(value).toContainText('4 entries')
  })

  test('sorted set: highest score first, 500 at a time with "Load more"', async ({ page }) => {
    const { tab, value } = await openBrowser(page)
    await openKey(page, tab, 'leaderboard:weekly')
    const table = value.getByRole('table', { name: 'Members' })
    await expect(table.getByRole('row').nth(1)).toHaveText(/9812\s*team:0000/)
    await expect(value).toContainText('1,204 members')
    await expect(value.getByText('Showing 500 of 1,204')).toBeVisible()
    await value.getByRole('button', { name: 'Load more' }).click()
    await expect(value.getByText('Showing 1,000 of 1,204')).toBeVisible()
    await value.getByRole('button', { name: 'Load more' }).click()
    await expect(value.getByText('All 1,204 members loaded')).toBeVisible()
    await expect(table).toHaveAttribute('aria-rowcount', '1205')
    // Only the rows near the viewport are in the page (the table is virtualised).
    expect(await table.getByRole('row').count()).toBeLessThan(120)
  })

  test('JSON module keys show as a tree or as text; a type without a viewer says so', async ({ page }) => {
    const { tab, value } = await openBrowser(page)
    await openKey(page, tab, 'config:pricing')
    await expect(value.getByLabel('Value as JSON')).toContainText('"currency": "CAD"')
    await expect(value.getByLabel('Value as JSON')).toContainText('"trial_days": 14')
    await value.getByRole('radio', { name: 'Text' }).click()
    await expect(value.getByLabel('Value as text')).toContainText('{"currency":"CAD","tiers":')

    await openKey(page, tab, 'metrics:cpu')
    await expect(value).toContainText('Dalan has no viewer for the type TSDB-TYPE yet.')
  })

  test('a key that is gone says so; Reload reads the value again; another database starts the tree again', async ({ page }) => {
    const { fake, tab, value } = await openBrowser(page)
    await openKey(page, tab, 'maintenance')
    await expect(value.getByLabel('Value as text')).toBeVisible()
    await value.getByRole('button', { name: 'Reload value' }).click()
    await expect.poll(async () => (await fake.calls('redis_key_info')).length).toBe(2)

    await fake.redisDropKey('maintenance')
    await value.getByRole('button', { name: 'Reload value' }).click()
    await expect(value.getByRole('alert')).toContainText('This key is gone. It expired or was deleted.')
    await value.getByRole('button', { name: 'Close' }).click()
    await expect(page.getByRole('heading', { name: 'Pick a key to see its value.' })).toBeVisible()

    await tab.getByLabel('Key pattern').fill('')
    await tab.getByLabel('Key pattern').press('Enter')
    await tab.getByRole('combobox', { name: 'Logical database' }).click()
    await page.getByRole('option', { name: 'db3 · 2 keys' }).click()
    await expect(item(tab, /^other:/)).toContainText('2')
    await expect(tab.getByText('All 2 keys scanned')).toBeVisible()
  })
})
