import { expect, test } from '../support/fixtures'
import type { Page } from '@playwright/test'
import { openApp } from './harness'

// D13 (replica/databases/restore-copy.md §3): S49 "Copy data", F32. The fake moves no rows; it plans from its
// made-up schema (every PostgreSQL database has the same tables) and keeps the gate's rules.
const nav = (page: Page) => page.getByRole('navigation', { name: 'Vault' })

async function addDb(page: Page, name: string, opts: { production?: boolean } = {}) {
  if (await page.getByRole('tab', { name: 'Vault' }).isVisible()) await page.getByRole('tab', { name: 'Vault' }).click()
  await nav(page).getByRole('link', { name: /Databases/ }).click()
  await page.getByRole('button', { name: 'New database' }).click()
  await page.getByTestId('new-database-dialog').getByRole('button', { name: 'Continue' }).click()
  const s = page.getByTestId('db-form-sheet')
  await s.getByLabel('Name', { exact: true }).fill(name)
  if (opts.production) await s.getByRole('radio', { name: 'Production' }).click()
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

/** Opens two databases and the Copy tab of the first; the second is the target. */
async function twoDbs(page: Page, opts: { targetProduction?: boolean } = {}) {
  await addDb(page, 'copy-target', { production: opts.targetProduction })
  const src = await addDb(page, 'app-prod')
  await src.getByTestId('sql-open-copy').click()
  const copy = src.getByTestId('copy-tab')
  await expect(copy.getByRole('table', { name: 'Objects to copy' })).toContainText('customers')
  return copy
}

test.describe('Copy between databases (S49, F32)', () => {
  test('tables that exist block “create”; drop and recreate is typed, then the report checks the rows', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    const copy = await twoDbs(page)
    await expect(copy.getByRole('combobox', { name: 'Target database' })).toHaveValue(/.+/)
    await copy.getByRole('checkbox', { name: 'Copy public.customers' }).check()
    await copy.getByRole('checkbox', { name: 'Copy public.orders' }).check()
    await copy.getByTestId('copy-review').click()
    const plan = copy.getByTestId('copy-plan')
    await expect(plan.locator('[data-part=blocked]')).toContainText('2 already in app: public.customers, public.orders')
    await expect(copy.getByTestId('copy-run')).toBeDisabled()
    await plan.getByRole('button', { name: 'Back' }).click()
    await copy.getByRole('radio', { name: 'Drop and recreate' }).check()
    await copy.getByTestId('copy-review').click()
    await expect(plan).toContainText('drop and create')
    await expect(copy.getByTestId('copy-run')).toBeDisabled()
    await copy.getByTestId('copy-typed').fill('copy-target')
    await copy.getByTestId('copy-run').click()
    const report = copy.getByTestId('copy-report')
    await expect(report.locator('[data-part=outcome]')).toContainText('Copied into app')
    await expect(report.getByRole('table', { name: 'Copy report' })).toContainText('1,200 on both sides')
    expect((await fake.calls()).includes('copy:app')).toBe(true)
  })

  test('into a new database it is a confirm; a renamed sample says so; a failed table can run again alone', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    const copy = await twoDbs(page)
    await copy.getByRole('checkbox', { name: 'Copy public.customers' }).check()
    await copy.getByRole('textbox', { name: 'Name at the target for customers' }).fill('customers_sample')
    await copy.getByRole('textbox', { name: 'Row limit for customers' }).fill('10')
    await copy.getByRole('checkbox', { name: 'Copy public.no_key' }).check()
    await copy.getByRole('checkbox', { name: 'Into a new database on that server' }).check()
    await copy.getByRole('textbox', { name: 'New database name' }).fill('scratch')
    await copy.getByRole('checkbox', { name: /Stop at the first table/ }).uncheck()
    await copy.getByTestId('copy-review').click()
    await expect(copy.getByTestId('copy-plan')).toContainText('public.customers_sample')
    await expect(copy.getByTestId('copy-typed')).toHaveCount(0)
    const sent = JSON.parse((await fake.calls('copy-request')).at(-1)!.slice('copy-request:'.length))
    expect(sent).toMatchObject({ objects: ['customers>customers_sample', 'no_key'], mode: 'create', newDb: 'scratch' })
    await copy.getByTestId('copy-run').click()
    const report = copy.getByTestId('copy-report')
    await expect(report.locator('[data-part=outcome]')).toContainText('Some tables failed')
    await expect(report).toContainText('duplicate key value')
    await copy.getByTestId('copy-again').click()
    await expect(copy.getByRole('checkbox', { name: 'Copy public.no_key' })).toBeChecked()
    await expect(copy.getByRole('checkbox', { name: 'Copy public.customers' })).not.toBeChecked()
  })

  test('Stop: the table being copied stops and the rest are skipped', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    await fake.restoreNext('slow')
    const copy = await twoDbs(page)
    await copy.getByRole('checkbox', { name: 'Copy public.orders' }).check()
    await copy.getByRole('checkbox', { name: 'Copy public.order_items' }).check()
    await copy.getByRole('checkbox', { name: 'Into a new database on that server' }).check()
    await copy.getByRole('textbox', { name: 'New database name' }).fill('scratch')
    await copy.getByTestId('copy-review').click()
    await copy.getByTestId('copy-run').click()
    await expect(copy.getByTestId('copy-progress')).toContainText('Copying public.orders (1 of 2)')
    await copy.getByTestId('copy-stop').click()
    const report = copy.getByTestId('copy-report')
    await expect(report.locator('[data-part=outcome]')).toContainText('Stopped.')
    await expect(report.getByRole('table', { name: 'Copy report' })).toContainText('skipped')
  })

  test('a production target must be Open first', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const copy = await twoDbs(page, { targetProduction: true })
    await copy.getByRole('combobox', { name: 'Target database' }).selectOption({ label: 'copy-target' })
    await copy.getByRole('checkbox', { name: 'Copy public.customers' }).check()
    await copy.getByTestId('copy-review').click()
    await expect(copy.locator('[data-part=error]')).toContainText('copy-target is a production database. To copy into it, switch it to Open first')
  })
})
