import { expect, test } from '../support/fixtures'
import type { Page } from '@playwright/test'
import { openApp } from './harness'

// D16 (replica/databases/restore-copy.md §9, E3): schema compare between open tabs and snapshots, the differences
// with both DDLs, and Sync through the target's gate. The fake has one fixed set of differences.
const nav = (page: Page) => page.getByRole('navigation', { name: 'Vault' })

async function addDb(page: Page, name: string) {
  if (await page.getByRole('tab', { name: 'Vault' }).isVisible()) await page.getByRole('tab', { name: 'Vault' }).click()
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

test.describe('Schema compare and sync (E3)', () => {
  test('two databases compare; drops are opt-in; a type change asks for the typed name; the script opens and saves', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    await addDb(page, 'stage-db')
    const tab = await addDb(page, 'app-prod')
    await tab.getByTestId('sql-open-compare').click()
    const cmp = tab.getByTestId('compare-tab')
    await expect(cmp.getByRole('combobox', { name: 'Source database' })).toHaveValue(/.+/)
    await expect(cmp.getByRole('combobox', { name: 'Target database' }).locator('option:checked')).toHaveText('stage-db')
    await expect(cmp.getByRole('combobox', { name: 'Source schema' })).toHaveValue('public')
    await cmp.getByTestId('compare-run').click()
    await expect(cmp.locator('[data-part=summary]')).toContainText('6 differences · 3 added · 2 removed · 1 changed')
    const list = cmp.getByRole('list', { name: 'Differences' })
    await expect(list.getByRole('checkbox', { name: 'Include column legacy' })).not.toBeChecked()
    await expect(list.getByRole('checkbox', { name: 'Include column id' })).toBeChecked()
    await list.getByRole('button', { name: /Changed column id/ }).click()
    await expect(cmp.getByLabel('Source DDL')).toHaveText('bigint NOT NULL')
    await expect(cmp.getByLabel('Target DDL')).toHaveText('integer NOT NULL')
    await list.getByRole('button', { name: 'customers', exact: true }).click()
    await expect(cmp.getByLabel('Target DDL')).toContainText('legacy text')

    // Sync: the type change makes it typed; drops weren't ticked.
    await cmp.getByTestId('compare-sync').click()
    const plan = cmp.getByTestId('compare-plan')
    await expect(plan.getByLabel('Sync script')).toContainText('ALTER TABLE customers ALTER COLUMN id TYPE bigint')
    await expect(plan.getByLabel('Sync script')).not.toContainText('DROP')
    await expect(plan).toContainText('4 statements · 1 drop or change data')
    expect(await fake.calls('schema-sync')).toContain('schema-sync:column:customers.id,column:customers.tier,index:customers.customers_tier_idx,table:orders')
    await expect(plan.getByTestId('compare-commit')).toBeDisabled()
    await plan.getByRole('textbox', { name: 'Type stage-db to confirm' }).fill('stage-db')
    await plan.getByTestId('compare-commit').click()
    await expect(cmp.locator('[data-part=report]')).toContainText('Ran 4 statements.')

    // The drops, ticked, go under their own heading; the script saves to a file and opens in the target's tab.
    await list.getByRole('checkbox', { name: 'Include column legacy' }).check()
    await cmp.getByRole('button', { name: 'Copy as migration…' }).click()
    await expect(cmp.getByRole('status').filter({ hasText: 'Saved migrate-public' })).toBeVisible()
    await cmp.getByRole('button', { name: 'Open in a query tab' }).click()
    const stage = page.getByTestId('sql-tab-stage-db')
    await expect(stage.getByRole('button', { name: 'Close Sync script' })).toBeVisible()
    await expect(stage.locator('.cm-content')).toContainText('-- Drops: these remove objects and the data in them.')
  })

  test('a snapshot can be either side; a snapshot target can\'t be synced; a side against itself has no differences', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await addDb(page, 'app-prod')
    await tab.getByTestId('sql-open-compare').click()
    const cmp = tab.getByTestId('compare-tab')
    await cmp.getByTestId('compare-run').click()
    await expect(cmp.locator('[data-part=summary]')).toContainText('No differences: app-prod · public matches app-prod · public.')
    const src = cmp.getByRole('region', { name: 'Source' })
    await src.getByRole('button', { name: 'Save a snapshot' }).click()
    await expect(cmp.getByRole('status').filter({ hasText: 'Saved a snapshot of app-prod · public.' })).toBeVisible()
    const tgt = cmp.getByRole('region', { name: 'Target' })
    await tgt.getByRole('radio', { name: 'A snapshot' }).check()
    await expect(tgt.getByRole('combobox', { name: 'Target snapshot' })).toContainText('app-prod · public')
    await cmp.getByTestId('compare-run').click()
    await expect(cmp.locator('[data-part=summary]')).toContainText('6 differences')
    await expect(cmp.getByTestId('compare-sync')).toBeDisabled()
    await tgt.getByRole('button', { name: 'Delete this target snapshot' }).click()
    await expect(tgt.getByRole('radio', { name: 'A snapshot' })).toBeDisabled()
  })
})
