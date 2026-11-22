import { expect, test } from '../support/fixtures'
import type { Page } from '@playwright/test'
import { openApp } from './harness'

// D2 of SQL and MongoDB (replica/databases/spec.md): opening a PostgreSQL database in a tab, the lazy object tree, and a
// table's Data view: filter, sort, columns, paging with the estimate, the cell panel and the linked rows (S36, F21).
// The fake backend plays the connection and serves one made-up schema (tests/fake/commands/sql.ts).
const nav = (page: Page) => page.getByRole('navigation', { name: 'Vault' })

async function openShop(page: Page) {
  await nav(page).getByRole('link', { name: /Databases/ }).click()
  await page.getByRole('button', { name: 'New database' }).click()
  await page.getByTestId('new-database-dialog').getByRole('button', { name: 'Continue' }).click()
  const s = page.getByTestId('db-form-sheet')
  await s.getByLabel('Name', { exact: true }).fill('app-prod')
  await s.getByRole('radio', { name: 'Production' }).click()
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

test.describe('A PostgreSQL tab (S36)', () => {
  test('F21: opening the database shows the steps, then the tree with the usual schema open and row estimates', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await expect(page.getByRole('tab', { name: /app-prod/ })).toBeVisible() // the app's own tab
    await expect(tab.getByText('PostgreSQL 16.4')).toBeVisible()
    await expect(tab.getByText('production', { exact: true })).toBeVisible()
    await expect(tab.getByRole('treeitem', { name: 'Schema public' })).toHaveAttribute('aria-expanded', 'true')
    await expect(tab.getByRole('treeitem', { name: 'Schema billing' })).toHaveAttribute('aria-expanded', 'false')
    const orders = tab.getByRole('treeitem', { name: 'table orders' })
    await expect(orders).toContainText('1.2k')
    await expect(tab.getByRole('treeitem', { name: 'view open_orders' })).toBeVisible()
    // The search box filters the tables that are loaded.
    await tab.getByLabel('Find a table').fill('item')
    await expect(orders).toHaveCount(0)
    await expect(tab.getByRole('treeitem', { name: 'table order_items' })).toBeVisible()
    // Opening the empty schema says so.
    await tab.getByLabel('Find a table').fill('')
    await tab.getByRole('treeitem', { name: 'Schema billing' }).click()
    await expect(tab.getByText('This schema has no tables yet.')).toBeVisible()
  })

  test('F21: a table opens on its first page with the estimate, pages on, counts exactly, and sorts by a header', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await tab.getByRole('treeitem', { name: 'table orders' }).click()
    const grid = tab.getByRole('grid', { name: 'public.orders rows' })
    await expect(grid.getByRole('columnheader', { name: 'Primary key id bigint' })).toBeVisible()
    await expect(grid.getByRole('row').nth(1)).toContainText('1')
    await expect(tab.getByTestId('sql-range')).toContainText('Rows 1–200 of 1,180 (estimate)')
    await tab.getByRole('button', { name: 'Next page' }).click()
    await expect(tab.getByTestId('sql-range')).toContainText('Rows 201–400')
    await tab.getByRole('button', { name: 'Previous page' }).click()
    await expect(tab.getByTestId('sql-range')).toContainText('Rows 1–200')
    await tab.getByRole('button', { name: 'Count exactly' }).click()
    await expect(tab.getByTestId('sql-range')).toContainText('of 1,200')
    // Sorting by total, twice: descending puts the biggest first.
    await grid.getByRole('button', { name: /^total/ }).click()
    await grid.getByRole('button', { name: /^total/ }).click()
    await expect(grid.getByRole('columnheader', { name: /^total/ })).toHaveAttribute('aria-sort', 'descending')
    await expect(grid.getByRole('row').nth(1)).toContainText('1500.00')
    await grid.getByRole('button', { name: /^total/ }).click()
    await expect(grid.getByRole('columnheader', { name: /^total/ })).toHaveAttribute('aria-sort', 'none')
  })

  test('F21: the Where box filters, a bad one says why, and the Columns menu hides a column', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await tab.getByRole('treeitem', { name: 'table orders' }).click()
    await tab.getByLabel('Where').fill('status = \'open\' and id <= 30')
    await tab.getByRole('button', { name: 'Apply' }).click()
    await expect(tab.getByTestId('sql-range')).toContainText('Rows 1–10')
    await tab.getByLabel('Where').fill('1 = 1; drop table orders')
    await tab.getByRole('button', { name: 'Apply' }).click()
    await expect(tab.getByRole('alert')).toContainText('That isn\'t a filter I can use')
    await tab.getByLabel('Where').fill('id > 100000')
    await tab.getByRole('button', { name: 'Apply' }).click()
    await expect(tab.getByText('No rows match this filter. Clear it?')).toBeVisible()
    await tab.getByLabel('Where').fill('')
    await tab.getByRole('button', { name: 'Apply' }).click()
    await tab.getByRole('button', { name: /Columns 5\/5/ }).click()
    await page.getByRole('menuitemcheckbox', { name: 'note' }).click()
    await page.keyboard.press('Escape')
    await expect(tab.getByRole('button', { name: /Columns 4\/5/ })).toBeVisible()
    await expect(tab.getByRole('grid').getByRole('columnheader', { name: /^note/ })).toHaveCount(0)
  })

  test('F21: the cell panel shows a value, and the linked rows open the other table filtered', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await tab.getByRole('treeitem', { name: 'table orders' }).click()
    const grid = tab.getByRole('grid', { name: 'public.orders rows' })
    await grid.getByRole('gridcell', { name: '1' }).first().click() // the id of the first row
    const panel = tab.getByRole('complementary', { name: 'Cell' })
    await expect(panel.getByText('bigint · row 1')).toBeVisible()
    await expect(panel.getByText('order_items rows point here')).toBeVisible()
    // Arrow keys move the selection and the panel follows.
    await grid.press('ArrowRight')
    await expect(panel.getByText('customer_id')).toBeVisible()
    await expect(panel.getByText('customers 2')).toBeVisible()
    await panel.getByRole('button', { name: 'Open row' }).click()
    await expect(tab.getByRole('button', { name: 'customers', exact: true })).toHaveAttribute('aria-current', 'true')
    await expect(tab.getByTestId('sql-table-customers').getByLabel('Where')).toHaveValue('"id" = 2')
    await expect(tab.getByTestId('sql-table-customers').getByTestId('sql-range')).toContainText('Rows 1–1')
    // Close the table tab again.
    await tab.getByRole('button', { name: 'Close customers' }).click()
    await expect(tab.getByRole('button', { name: 'customers', exact: true })).toHaveCount(0)
  })

  test('F21: a refused Apply keeps the page the table was on, and the first arrow key selects the first cell', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await tab.getByRole('treeitem', { name: 'table orders' }).click()
    await tab.getByRole('button', { name: 'Next page' }).click()
    await expect(tab.getByTestId('sql-range')).toContainText('Rows 201–400')
    await tab.getByLabel('Where').fill('1 = 1; drop table orders')
    await tab.getByRole('button', { name: 'Apply' }).click()
    await expect(tab.getByRole('alert')).toContainText('That isn\'t a filter I can use')
    await expect(tab.getByTestId('sql-range')).toContainText('Rows 201–400')
    await tab.getByRole('button', { name: 'Next page' }).click()
    await expect(tab.getByTestId('sql-range')).toContainText('Rows 401–600')
    const grid = tab.getByRole('grid', { name: 'public.orders rows' })
    await grid.focus()
    await grid.press('ArrowDown')
    await expect(tab.getByRole('complementary', { name: 'Cell' }).getByText('row 401')).toBeVisible()
    // A tree: → opens a closed schema, ← closes it.
    const billing = tab.getByRole('treeitem', { name: 'Schema billing' })
    await billing.focus()
    await billing.press('ArrowRight')
    await expect(billing).toHaveAttribute('aria-expanded', 'true')
    await billing.press('ArrowLeft')
    await expect(billing).toHaveAttribute('aria-expanded', 'false')
  })

  test('F21: a table with no key says rows can not be matched, and a view opens too', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await tab.getByRole('treeitem', { name: 'table no_key' }).click()
    await expect(tab.getByText('This table has no key, so rows can\'t be matched safely.')).toBeVisible()
    await tab.getByRole('treeitem', { name: 'view open_orders' }).click()
    await expect(tab.getByRole('grid', { name: 'public.open_orders rows' })).toBeVisible()
  })

  test('S33: closing the tab closes its connection', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await expect(tab).toBeVisible()
    await page.getByRole('button', { name: 'Close app-prod' }).click()
    await expect(page.getByTestId('sql-tab-app-prod')).toHaveCount(0)
    expect((await fake.calls()).some(c => c.startsWith('db_close:'))).toBe(true)
  })
})
