import { expect, test } from '../support/fixtures'
import type { Page } from '@playwright/test'
import { openApp } from './harness'

// D7 of SQL and MongoDB (replica/databases/spec.md): MySQL and MariaDB tabs browse and query like PostgreSQL ones, through the
// same gate; the views that are not built for them yet are not offered. The fake backend answers (it serves one made-up schema).
const nav = (page: Page) => page.getByRole('navigation', { name: 'Vault' })

async function openMy(page: Page, engine: 'MySQL' | 'MariaDB') {
  await nav(page).getByRole('link', { name: /Databases/ }).click()
  await page.getByRole('button', { name: 'New database' }).click()
  await page.getByTestId('new-database-dialog').getByRole('radio', { name: new RegExp(engine) }).click()
  await page.getByTestId('new-database-dialog').getByRole('button', { name: 'Continue' }).click()
  const s = page.getByTestId('db-form-sheet')
  await s.getByLabel('Name', { exact: true }).fill('legacy-crm')
  await s.getByLabel('Address').fill('db.internal')
  await s.getByLabel('Username').fill('app')
  await s.getByRole('button', { name: 'Save' }).click()
  await expect(s).toHaveCount(0)
  await page.getByRole('row', { name: /legacy-crm/ }).dblclick()
  const tab = page.getByTestId('sql-tab-legacy-crm')
  await expect(tab.getByRole('tree', { name: 'Schemas and tables' })).toBeVisible()
  return tab
}

for (const engine of ['MySQL', 'MariaDB'] as const) {
  test.describe(`A ${engine} tab (D7)`, () => {
    test(`opens, names the server, browses a table and runs a query`, async ({ page }) => {
      await openApp(page, { scenario: 'team', unlocked: true })
      const tab = await openMy(page, engine)
      await expect(tab).toContainText(engine === 'MySQL' ? 'MySQL 8.4.2' : 'MariaDB 11.4.2')
      await tab.getByTestId('sql-object-orders').click()
      await expect(tab.getByRole('grid', { name: 'public.orders rows' })).toBeVisible()
      await tab.getByRole('button', { name: 'New query' }).click()
      await tab.getByRole('textbox', { name: /SQL$/ }).click()
      await page.keyboard.insertText('select * from customers')
      await tab.getByTestId('sql-run').click()
      await expect(tab.getByRole('button', { name: /Result 1 · 50 rows/ })).toBeVisible()
    })

    test('a change goes through the same confirmation, and views not built for it yet are not offered', async ({ page }) => {
      await openApp(page, { scenario: 'team', unlocked: true })
      const tab = await openMy(page, engine)
      await tab.getByRole('button', { name: 'New query' }).click()
      await tab.getByRole('textbox', { name: /SQL$/ }).click()
      await page.keyboard.insertText('update customers set name = \'x\' where id = 1')
      await tab.getByTestId('sql-run').click()
      await expect(page.getByTestId('sql-confirm')).toContainText('This will change 1 row.')
      await page.getByTestId('sql-confirm').getByRole('button', { name: 'Cancel' }).click()
      // Not offered: explain, manual transactions, diagram, activity, transfer, structure.
      await expect(tab.getByTestId('sql-explain')).toHaveCount(0)
      await expect(tab.getByTestId('sql-manual')).toHaveCount(0)
      await expect(tab.getByTestId('sql-new-diagram')).toHaveCount(0)
      await expect(tab.getByTestId('sql-open-activity')).toHaveCount(0)
      await tab.getByTestId('sql-object-orders').click()
      await expect(tab.getByRole('group', { name: 'View' })).toHaveCount(0)
    })
  })
}
