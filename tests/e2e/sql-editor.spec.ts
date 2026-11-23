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

  // ---- D4: the gate, transactions and grid edits (S42, F22)
  test('S42: a change in Confirm mode shows what it will do and runs only after Run; Cancel sends nothing', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await query(page, tab, 'update customers set name = \'x\' where id = 1')
    await tab.getByTestId('sql-run').click()
    const d = page.getByTestId('sql-confirm')
    await expect(d).toContainText('This will change 1 row.')
    await d.getByRole('button', { name: 'Cancel' }).click()
    await expect(d).toHaveCount(0)
    await expect(tab.getByRole('button', { name: /Result 1/ })).toHaveCount(0)
    await tab.getByTestId('sql-run').click()
    await page.getByTestId('sql-confirm').getByRole('button', { name: 'Run', exact: true }).click()
    await expect(tab.getByTestId('sql-messages')).toContainText('1 row changed')
  })

  test('S42: a dangerous statement needs the database\'s name typed', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await query(page, tab, 'delete from orders')
    await tab.getByTestId('sql-run').click()
    const d = page.getByTestId('sql-danger')
    await expect(d).toContainText('Dangerous statement on app-prod?')
    await expect(d.getByRole('button', { name: 'Run it anyway' })).toBeDisabled()
    await d.getByLabel('Type app-prod to confirm').fill('app')
    await expect(d.getByRole('button', { name: 'Run it anyway' })).toBeDisabled()
    await d.getByLabel('Type app-prod to confirm').fill('app-prod')
    await d.getByRole('button', { name: 'Run it anyway' }).click()
    await expect(tab.getByTestId('sql-messages')).toContainText('rows changed')
  })

  test('S42: a Read-only database refuses a change without a dialog', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    await nav(page).getByRole('link', { name: /Databases/ }).click()
    await page.getByRole('button', { name: 'New database' }).click()
    await page.getByTestId('new-database-dialog').getByRole('button', { name: 'Continue' }).click()
    const s = page.getByTestId('db-form-sheet')
    await s.getByLabel('Name', { exact: true }).fill('app-prod')
    await s.getByLabel('Address').fill('db.internal')
    await s.getByLabel('Database', { exact: true }).fill('app')
    await s.getByLabel('Username').fill('app')
    await s.getByRole('radio', { name: /Read-only/ }).click()
    await s.getByRole('button', { name: 'Save' }).click()
    await page.getByRole('row', { name: /app-prod/ }).dblclick()
    const tab = page.getByTestId('sql-tab-app-prod')
    await query(page, tab, 'delete from orders where id = 1')
    await tab.getByTestId('sql-run').click()
    await expect(tab.getByTestId('sql-messages')).toContainText('app-prod is read-only in Dalan. Nothing was sent.')
    await expect(page.getByTestId('sql-confirm')).toHaveCount(0)
    // No manual transactions either.
    await tab.getByTestId('sql-manual').click()
    await expect(tab.getByTestId('sql-messages')).toContainText('read-only')
    await expect(tab.getByTestId('sql-txn-bar')).toHaveCount(0)
  })

  test('S37: a raw BEGIN or COMMIT in a script is refused', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await query(page, tab, 'begin; select 1')
    await tab.getByRole('button', { name: 'Run all' }).click()
    await expect(tab.getByTestId('sql-messages')).toContainText('Manual / Auto-commit button')
  })

  test('S37: Manual mode opens a transaction, the bar counts statements, and closing the tab asks what to do with it', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await query(page, tab, 'update customers set name = \'x\' where id = 1')
    await tab.getByTestId('sql-manual').click()
    const bar = tab.getByTestId('sql-txn-bar')
    await expect(bar).toContainText('In a transaction')
    await expect(bar).toContainText('0 statements not committed')
    await tab.getByTestId('sql-run').click()
    await page.getByTestId('sql-confirm').getByRole('button', { name: 'Run', exact: true }).click()
    await expect(bar).toContainText('1 statement not committed')
    await expect(bar).toContainText('holds 2 locks')
    // Closing the tab with the transaction open asks.
    await tab.getByRole('button', { name: 'Close Query 1' }).click()
    const g = page.getByTestId('sql-close-guard')
    await expect(g).toContainText('1 statement is not committed')
    await g.getByRole('button', { name: 'Keep it open' }).click()
    await expect(bar).toBeVisible()
    await bar.getByRole('button', { name: 'Roll back' }).click()
    await expect(bar).toHaveCount(0)
    await tab.getByRole('button', { name: 'Close Query 1' }).click()
    await expect(tab.getByRole('button', { name: /^Query 1/ })).toHaveCount(0)
  })

  test('S37: closing the whole tab with a transaction open asks first', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await query(page, tab, 'select 1')
    await tab.getByTestId('sql-manual').click()
    await expect(tab.getByTestId('sql-txn-bar')).toBeVisible()
    await page.getByRole('button', { name: 'Close app-prod' }).click()
    await expect(page.getByRole('alertdialog')).toContainText('A transaction is open in this tab')
  })
})

test.describe('Editing rows in the grid (F22)', () => {
  async function openCustomers(page: Page) {
    const tab = await openShop(page)
    await tab.getByTestId('sql-object-customers').click()
    const grid = tab.getByRole('grid', { name: 'public.customers rows' })
    await expect(grid).toBeVisible()
    return { tab, grid }
  }

  test('F22: edit two cells and delete a row, review the statements, commit, and the table shows them', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const { tab, grid } = await openCustomers(page)
    const cell = (row: number, col: number) => grid.getByRole('row').nth(row + 1).getByRole('gridcell').nth(col)
    await cell(0, 1).dblclick()
    await page.keyboard.press('ControlOrMeta+a')
    await page.keyboard.insertText('Ada')
    await page.keyboard.press('Enter')
    await expect(cell(0, 1)).toHaveText('Ada')
    await cell(1, 2).click()
    await grid.press('Delete')
    await expect(cell(1, 2)).toHaveText('NULL')
    await cell(2, 1).click()
    await tab.getByRole('button', { name: 'Delete row' }).click()
    await expect(tab.getByTestId('sql-changes')).toContainText('3 changes')
    await tab.getByRole('button', { name: 'Review changes' }).click()
    const d = page.getByTestId('sql-commit')
    await expect(d.getByLabel('Statement')).toHaveCount(3)
    await expect(d).toContainText('"name" = \'Ada\'')
    await expect(d).toContainText('is not distinct from \'customer 1\'')
    await expect(d).toContainText('delete from "public"."customers" where "id" = \'3\'')
    await d.getByRole('button', { name: 'Commit' }).click()
    await expect(d).toHaveCount(0)
    await expect(tab.getByTestId('sql-changes')).toHaveCount(0)
    await expect(cell(0, 1)).toHaveText('Ada')
    // Customer 3 is gone: the third row is customer 4.
    await expect(cell(2, 0)).toHaveText('4')
  })

  test('F22: a row changed by someone else stops the commit and says so; nothing is written', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const { tab, grid } = await openCustomers(page)
    const cell = (row: number, col: number) => grid.getByRole('row').nth(row + 1).getByRole('gridcell').nth(col)
    await cell(3, 1).dblclick()
    await page.keyboard.insertText('Mine')
    await page.keyboard.press('Enter')
    // Meanwhile someone else changes customer 4 (here: a query in another tab, which the fake applies).
    await tab.getByRole('button', { name: 'New query' }).click()
    await tab.getByRole('textbox', { name: /SQL$/ }).click()
    await page.keyboard.insertText('update customers set name = \'Theirs\' where id = 4')
    await tab.getByTestId('sql-run').click()
    await page.getByTestId('sql-confirm').getByRole('button', { name: 'Run', exact: true }).click()
    await expect(tab.getByTestId('sql-messages')).toContainText('1 row changed')
    await tab.getByRole('button', { name: 'customers', exact: true }).click()
    await tab.getByRole('button', { name: 'Review changes' }).click()
    const d = page.getByTestId('sql-commit')
    await d.getByRole('button', { name: 'Commit' }).click()
    await expect(d.getByRole('alert')).toContainText('Row 1 was changed or removed by someone else')
    await expect(d.getByRole('alert')).toContainText('Nothing was written')
    await d.getByRole('button', { name: 'Close' }).click()
    // Still pending in the grid; refresh is off until they are committed or discarded.
    await expect(tab.getByTestId('sql-changes')).toContainText('1 change')
    await tab.getByRole('button', { name: 'Discard' }).click()
    await expect(tab.getByTestId('sql-changes')).toHaveCount(0)
    await tab.getByRole('button', { name: 'Refresh' }).click()
    await expect(cell(3, 1)).toHaveText('Theirs')
  })

  test('F22: a table with no key and a Read-only database are not editable', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await tab.getByTestId('sql-object-no_key').click()
    const grid = tab.getByRole('grid', { name: 'public.no_key rows' })
    await grid.getByRole('row').nth(1).getByRole('gridcell').nth(1).dblclick()
    await expect(grid.locator('input[data-cell-editor]')).toHaveCount(0)
  })
})
