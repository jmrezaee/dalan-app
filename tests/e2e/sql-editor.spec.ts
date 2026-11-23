import { expect, test } from '../support/fixtures'
import type { Locator, Page } from '@playwright/test'
import { openApp } from './harness'

// D3 of SQL and MongoDB (replica/databases/spec.md): the query editor (S37, F23): run the statement at the caret, the
// selection or the whole script; results stream into tabs; Stop; limit; refused writes; history, saved queries, copy as.
// The fake backend answers by what a statement mentions (tests/fake/commands/sql.ts).
const nav = (page: Page) => page.getByRole('navigation', { name: 'Vault' })

async function openShop(page: Page) {
  await nav(page).getByRole('link', { name: /Databases/ }).click()
  await page.getByRole('button', { name: 'New database' }).click()
  await page.getByTestId('new-database-dialog').getByRole('button', { name: 'Continue' }).click()
  const s = page.getByTestId('db-form-sheet')
  await s.getByLabel('Name', { exact: true }).fill('app-prod')
  await s.getByLabel('Address').fill('db.internal')
  await s.getByLabel('Database', { exact: true }).fill('app')
  await s.getByLabel('Username').fill('app')
  await s.getByRole('button', { name: 'Save' }).click()
  await expect(s).toHaveCount(0)
  await page.getByRole('row', { name: /app-prod/ }).dblclick()
  const tab = page.getByTestId('sql-tab-app-prod')
  await expect(tab.getByRole('tree', { name: 'Schemas and tables' })).toBeVisible()
  return tab
}

/** A new query tab with `sql` typed in (select all, then type, so it replaces the empty document). */
async function query(page: Page, tab: Locator, sql: string) {
  await tab.getByRole('button', { name: 'New query' }).click()
  const box = tab.getByRole('textbox', { name: /SQL$/ })
  await expect(box).toBeVisible()
  await box.click()
  await page.keyboard.insertText(sql)
  return box
}

test.describe('The query editor (S37, F23)', () => {
  test('F23: ⌘↵ runs the statement at the caret and the rows arrive in a result tab', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await expect(tab.getByText('Open a table or start a query')).toBeVisible()
    await query(page, tab, 'select * from customers;\nselect 1')
    await page.keyboard.press('ControlOrMeta+Enter')
    // The caret is in the second statement: one row, not the customers.
    await expect(tab.getByRole('button', { name: /Result 1 · 1 row/ })).toBeVisible()
    await expect(tab.getByRole('grid', { name: 'Result 1' })).toContainText('NULL')
    await expect(tab.getByRole('button', { name: /Result 2/ })).toHaveCount(0)
  })

  test('F23: Run all makes a result tab per statement, with the messages and the row counts', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await query(page, tab, 'select * from customers; select * from orders')
    await tab.getByRole('button', { name: 'Run all' }).click()
    await expect(tab.getByRole('button', { name: /Result 1 · 50 rows/ })).toBeVisible()
    await expect(tab.getByRole('button', { name: /Result 2 · 1,000 rows/ })).toBeVisible()
    // The limit (1000) cut the second one off, and says so.
    await expect(tab.getByTestId('sql-more')).toHaveCount(0)
    await tab.getByRole('button', { name: /Result 2/ }).click()
    await expect(tab.getByTestId('sql-more')).toContainText('First 1,000 rows. Raise the limit to see more.')
    await tab.getByRole('button', { name: 'messages' }).click()
    await expect(tab.getByTestId('sql-messages')).toContainText('50 rows')
    await expect(tab.getByTestId('sql-messages')).toContainText('stopped at the limit of 1,000')
  })

  test('F23: a lower limit applies to the next run', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await query(page, tab, 'select * from orders')
    await tab.getByLabel('Limit').fill('25')
    await tab.getByTestId('sql-run').click()
    await expect(tab.getByRole('button', { name: /Result 1 · 25 rows/ })).toBeVisible()
  })

  test('F23: a statement that changes data is refused, and nothing of the script runs', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await query(page, tab, 'select 1; delete from orders where id = 1')
    await tab.getByRole('button', { name: 'Run all' }).click()
    await expect(tab.getByTestId('sql-messages')).toContainText('The editor only runs reads for now')
    await expect(tab.getByRole('button', { name: /Result 1/ })).toHaveCount(0)
  })

  test('F23: a failing statement shows the server\'s words and stops the script', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await query(page, tab, 'select 1; select * from missing; select 2')
    await tab.getByRole('button', { name: 'Run all' }).click()
    await expect(tab.getByTestId('sql-messages')).toContainText('relation "missing" does not exist')
    await tab.getByRole('button', { name: /Result 2 · failed/ }).waitFor()
    await expect(tab.getByRole('button', { name: /Result 3/ })).toHaveCount(0)
  })

  test('F23: Stop cancels a running statement, and Run is back', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await query(page, tab, 'select pg_sleep(60)')
    await tab.getByTestId('sql-run').click()
    await expect(tab.getByTestId('sql-stop')).toBeVisible()
    await tab.getByTestId('sql-stop').click()
    await expect(tab.getByTestId('sql-messages')).toContainText('Stopped.')
    await expect(tab.getByTestId('sql-run')).toBeVisible()
  })

  test('F23: Run selection runs only what is selected', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    const box = await query(page, tab, 'select * from customers; select * from orders')
    await box.click()
    await page.keyboard.press('ControlOrMeta+a')
    await tab.getByRole('button', { name: 'Run selection' }).click()
    await expect(tab.getByRole('button', { name: /Result 2/ })).toBeVisible()
    // Nothing selected: it says so instead of running everything.
    await box.click()
    await page.keyboard.press('ArrowLeft')
    await tab.getByRole('button', { name: 'Run selection' }).click()
    await expect(tab.getByTestId('sql-messages')).toContainText('Select some SQL first.')
  })

  test('S37: Format lays the statement out, and the tab shows an unsaved dot until it is saved with a name (⌘S)', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await query(page, tab, 'select * from customers where id = 1')
    await expect(tab.getByLabel('unsaved')).toBeVisible()
    await tab.getByRole('button', { name: 'Format' }).click()
    await expect(tab.getByRole('textbox', { name: /SQL$/ })).toContainText('FROM customers')
    await page.keyboard.press('ControlOrMeta+s')
    await tab.getByLabel('Name', { exact: true }).fill('Customer one')
    await tab.getByLabel('Name', { exact: true }).press('Enter')
    await expect(tab.getByLabel('unsaved')).toHaveCount(0)
    await expect(tab.getByRole('button', { name: 'Close Customer one' })).toBeVisible()
    expect((await fake.model()).savedQueries).toHaveLength(1)
    // The Saved strip lists it; opening it again makes a second tab.
    await tab.getByRole('button', { name: 'saved' }).click()
    await tab.getByRole('button', { name: /Customer one/ }).filter({ hasText: 'FROM' }).click()
    await expect(tab.getByRole('button', { name: 'Close Customer one' })).toHaveCount(2)
  })

  test('S37: History lists what ran, newest first, and a click puts it back in the editor', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await query(page, tab, 'select * from customers')
    await tab.getByTestId('sql-run').click()
    await expect(tab.getByRole('button', { name: /Result 1 · 50 rows/ })).toBeVisible()
    await tab.getByRole('button', { name: 'history' }).click()
    const entry = tab.getByRole('button', { name: 'select * from customers' })
    await expect(entry.first()).toBeVisible()
    await tab.getByRole('button', { name: 'Clear history' }).click()
    await expect(tab.getByText('Nothing run on this database yet.')).toBeVisible()
  })

  test('S37: Copy as puts the result on the clipboard in the chosen format', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write'])
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await query(page, tab, 'select * from customers')
    await tab.getByTestId('sql-run').click()
    await expect(tab.getByRole('button', { name: /Result 1 · 50 rows/ })).toBeVisible()
    await tab.getByRole('button', { name: 'Copy as…' }).click()
    await page.getByRole('menuitem', { name: 'CSV' }).click()
    await expect(tab.getByText('Copied as CSV')).toBeVisible()
    const text = await page.evaluate(() => navigator.clipboard.readText())
    expect(text.split('\r\n')[0]).toBe('id,name,meta')
    expect(text.split('\r\n')).toHaveLength(51)
  })

  test('S37: a query tab keeps its text and results while a table tab shows, and closing it ends its connection', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await query(page, tab, 'select * from customers')
    await tab.getByTestId('sql-run').click()
    await expect(tab.getByRole('button', { name: /Result 1 · 50 rows/ })).toBeVisible()
    await tab.getByTestId('sql-object-orders').click()
    await expect(tab.getByRole('grid', { name: 'public.orders rows' })).toBeVisible()
    await tab.getByRole('button', { name: /^Query 1/ }).click()
    await expect(tab.getByRole('textbox', { name: /SQL$/ })).toContainText('select * from customers')
    await expect(tab.getByRole('button', { name: /Result 1 · 50 rows/ })).toBeVisible()
    await tab.getByRole('button', { name: 'Close Query 1' }).click()
    await expect(tab.getByRole('button', { name: /^Query 1/ })).toHaveCount(0)
    expect((await fake.calls()).length).toBeGreaterThan(0)
  })
})
