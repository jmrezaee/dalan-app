import { expect, test } from '../support/fixtures'
import type { Page } from '@playwright/test'
import { openApp } from './harness'

// D17 (replica/databases/restore-copy.md §9, E6): many tables out to a folder or zip, a folder or zip of files back
// in (matched by name, mapped, or a reviewed CREATE TABLE), and MongoDB import. The fake writes and reads nothing.
const nav = (page: Page) => page.getByRole('navigation', { name: 'Vault' })

async function addDb(page: Page, name: string) {
  await nav(page).getByRole('link', { name: /Databases/ }).click()
  await page.getByRole('button', { name: 'New database' }).click()
  await page.getByTestId('new-database-dialog').getByRole('button', { name: 'Continue' }).click()
  const s = page.getByTestId('db-form-sheet')
  await s.getByLabel('Name', { exact: true }).fill(name)
  await s.getByLabel('Address').fill(`${name}.internal`)
  await s.getByLabel('Database', { exact: true }).fill('app')
  await s.getByLabel('Username').fill('app')
  await s.getByRole('button', { name: 'Save' }).click()
  await expect(s).toHaveCount(0)
  await page.getByRole('row', { name: new RegExp(name) }).dblclick()
  const tab = page.getByTestId(`sql-tab-${name}`)
  await expect(tab.getByRole('tree', { name: 'Schemas and tables' })).toBeVisible()
  return tab
}

async function openMongo(page: Page) {
  await nav(page).getByRole('link', { name: /Databases/ }).click()
  await page.getByRole('button', { name: 'New database' }).click()
  await page.getByTestId('new-database-dialog').getByRole('radio', { name: /MongoDB/ }).click()
  await page.getByTestId('new-database-dialog').getByRole('button', { name: 'Continue' }).click()
  const s = page.getByTestId('db-form-sheet')
  await s.getByLabel('Name', { exact: true }).fill('docs-prod')
  await s.getByLabel('Members').fill('mongo.internal:27017')
  await s.getByLabel('Username').fill('app')
  await s.getByRole('button', { name: 'Save' }).click()
  await expect(s).toHaveCount(0)
  await page.getByRole('row', { name: /docs-prod/ }).dblclick()
  const tab = page.getByTestId('mongo-tab-docs-prod')
  await expect(tab.getByRole('tree', { name: 'Databases and collections' })).toBeVisible()
  return tab
}

test.describe('Bulk export and import (E6)', () => {
  test('many tables export into a zip with masking; a folder of files imports, one of them into a new table', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await addDb(page, 'app-prod')
    await tab.getByTestId('sql-open-transfer').click()
    const ex = tab.getByTestId('bulk-export')
    await ex.getByRole('checkbox', { name: 'public.customers' }).click()
    await ex.getByRole('checkbox', { name: 'public.orders' }).click()
    await ex.getByRole('combobox', { name: 'Format of every file' }).selectOption('json')
    await ex.locator('summary').click()
    await ex.getByRole('button', { name: 'Add a rule' }).click()
    await ex.getByRole('textbox', { name: 'Masked column 1 of the bulk export' }).fill('email')
    await ex.getByRole('radio', { name: 'A .zip file' }).check()
    await ex.getByTestId('bulk-export-run').click()
    await expect(ex.locator('[data-part=bulk-done]')).toContainText('2 files, 20 rows, and manifest.json')
    expect(await fake.calls('bulk-export')).toContain('bulk-export:zip:json:public.customers,public.orders:ddl=true:masks=email=fake_email')

    const im = tab.getByTestId('bulk-import')
    await im.getByRole('button', { name: 'Choose a folder…' }).click()
    await expect(im).toContainText('exported by Dalan from app-prod')
    await expect(im).toContainText('not read: notes.pdf')
    const loyalty = im.getByTestId('bulk-file-loyalty.csv')
    await expect(loyalty.locator('[data-part=where]')).toHaveText('new table')
    await expect(im.getByTestId('bulk-file-public.customers.csv').locator('[data-part=where]')).toHaveText('existing table')
    await loyalty.getByRole('button', { name: 'CREATE TABLE' }).click()
    const ddl = loyalty.getByRole('textbox', { name: 'CREATE TABLE for loyalty.csv' })
    await expect(ddl).toHaveValue(/CREATE TABLE public\.loyalty/)
    await im.getByRole('checkbox', { name: 'Import public.orders.jsonl' }).click()
    await im.getByTestId('bulk-import-review').click()
    const plan = im.getByTestId('bulk-import-plan')
    await expect(plan).toContainText('Load 2 files (50 rows) into app-prod, creating 1 tables first?')
    expect((await fake.calls('bulk-import'))[0]).toBe('bulk-import:public.customers.csv>public.customers,loyalty.csv>public.loyalty+create')
    await plan.getByTestId('bulk-import-commit').click()
    await expect(im.getByRole('table', { name: 'Import report' })).toContainText('public.loyalty (new)')
  })

  test('a MongoDB file goes into a collection: insert reports what was refused; replace asks for the name', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openMongo(page)
    await tab.getByTestId('mongo-open-transfer').click()
    const mi = tab.getByTestId('mongo-import')
    await mi.getByRole('button', { name: 'Choose a file…' }).click()
    await expect(mi.locator('[data-part=mg-preview]')).toHaveText('3 documents in JSON lines.')
    await expect(mi.getByRole('textbox', { name: 'Collection to import into' })).toHaveValue('items')
    await mi.getByTestId('mongo-import-review').click()
    await mi.getByTestId('mongo-import-commit').click()
    await expect(mi.locator('[data-part=mg-report]')).toContainText('inserted 2 documents. 1 refused: E11000')
    await mi.getByRole('radio', { name: /Upsert by/ }).check()
    await mi.getByRole('combobox', { name: 'Field to upsert by' }).selectOption('sku')
    await mi.getByTestId('mongo-import-review').click()
    await mi.getByTestId('mongo-import-commit').click()
    await expect(mi.locator('[data-part=mg-report]')).toContainText('upserted 1, replaced 2')
    await mi.getByRole('radio', { name: /Replace the collection/ }).check()
    await mi.getByTestId('mongo-import-review').click()
    const plan = mi.getByTestId('mongo-import-plan')
    await expect(plan.getByTestId('mongo-import-commit')).toBeDisabled()
    await plan.getByRole('textbox', { name: 'Type docs-prod to confirm' }).fill('docs-prod')
    await plan.getByTestId('mongo-import-commit').click()
    await expect(mi.locator('[data-part=mg-report]')).toContainText('Removed 40, inserted 3 documents.')
    expect(await fake.calls('mongo-import')).toEqual([expect.stringMatching(/:insert$/), expect.stringMatching(/:upsert:sku$/), expect.stringMatching(/:replace$/)])
  })
})
