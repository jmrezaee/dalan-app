import { expect, test } from '../support/fixtures'
import type { Page } from '@playwright/test'
import { openApp } from './harness'

// D6 of SQL and MongoDB (replica/databases/spec.md): activity with blocking chains, cancel and end through the gate
// (S41, F25), and import, export, backup and restore (S43, F26). The fake backend answers.
const nav = (page: Page) => page.getByRole('navigation', { name: 'Vault' })

async function openShop(page: Page, opts: { production?: boolean, mode?: RegExp } = {}) {
  await nav(page).getByRole('link', { name: /Databases/ }).click()
  await page.getByRole('button', { name: 'New database' }).click()
  await page.getByTestId('new-database-dialog').getByRole('button', { name: 'Continue' }).click()
  const s = page.getByTestId('db-form-sheet')
  await s.getByLabel('Name', { exact: true }).fill('app-prod')
  if (opts.production) await s.getByRole('radio', { name: 'Production' }).click()
  await s.getByLabel('Address').fill('db.internal')
  await s.getByLabel('Database', { exact: true }).fill('app')
  await s.getByLabel('Username').fill('app')
  if (opts.mode) await s.getByRole('radio', { name: opts.mode }).click()
  await s.getByRole('button', { name: 'Save' }).click()
  await expect(s).toHaveCount(0)
  await page.getByRole('row', { name: /app-prod/ }).dblclick()
  const tab = page.getByTestId('sql-tab-app-prod')
  await expect(tab.getByRole('tree', { name: 'Schemas and tables' })).toBeVisible()
  return tab
}

test.describe('Activity (S41, F25)', () => {
  test('S41: tiles, sessions with their state, and the blocking chain; Dalan\'s own session has no buttons', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await tab.getByTestId('sql-open-activity').click()
    const a = tab.getByTestId('sql-activity')
    await expect(a.locator('[data-part=tiles]')).toContainText('86')
    await expect(a.locator('[data-part=tiles]')).toContainText('of 200')
    await expect(a.locator('[data-part=tiles]')).toContainText('99.4%')
    await expect(a.getByTestId('session-18342')).toContainText('update payments')
    await expect(a.getByTestId('session-18290')).toContainText('idle in transaction')
    // Idle sessions older than a minute are hidden until asked for.
    await expect(a.getByTestId('session-18400')).toHaveCount(0)
    await a.getByLabel('Hide idle').uncheck()
    await expect(a.getByTestId('session-18400')).toBeVisible()
    await expect(a.getByTestId('session-18377')).toContainText('you (Dalan)')
    await expect(a.getByTestId('session-18377').getByRole('button')).toHaveCount(0)
    await expect(a.locator('[data-part=chains]')).toContainText('1 blocking chain')
    await expect(a.locator('[data-part=chains]')).toContainText('18290')
    await a.getByLabel('Filter sessions').fill('payments')
    await expect(a.getByTestId('session-18298')).toHaveCount(0)
    await a.getByRole('button', { name: 'Locks' }).click()
    await expect(a.getByRole('region', { name: 'Locks' })).toContainText('RowShareLock')
    await a.getByRole('button', { name: 'Replication' }).click()
    await expect(a.getByRole('region', { name: 'Replication' })).toContainText('streaming')
  })

  test('S41: ending a blocker asks first, and its waiters are free afterwards', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await tab.getByTestId('sql-open-activity').click()
    const a = tab.getByTestId('sql-activity')
    await a.getByRole('button', { name: 'End session 18290' }).click()
    const d = page.getByTestId('session-confirm')
    await expect(d).toContainText('End session 18290?')
    await expect(d).toContainText('rolled back')
    await d.getByRole('button', { name: 'Cancel' }).click()
    await expect(a.getByTestId('session-18290')).toBeVisible()
    await a.getByRole('button', { name: 'End session 18290' }).click()
    await page.getByTestId('session-confirm').getByRole('button', { name: 'End session' }).click()
    await expect(a.locator('[data-part=note]')).toContainText('Ended session 18290.')
    await expect(a.getByTestId('session-18290')).toHaveCount(0)
    await expect(a.locator('[data-part=chains]')).toHaveCount(0)
  })

  test('S41: on a production database ending a session needs the database\'s name', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page, { production: true })
    await tab.getByTestId('sql-open-activity').click()
    await tab.getByRole('button', { name: 'Cancel query 18342' }).click()
    const d = page.getByTestId('session-confirm')
    await expect(d.getByRole('button', { name: 'Cancel query' })).toBeDisabled()
    await d.getByLabel('Type app-prod to confirm').fill('app-prod')
    await d.getByRole('button', { name: 'Cancel query' }).click()
    await expect(tab.getByTestId('sql-activity').locator('[data-part=note]')).toContainText('Cancelled the query of session 18342.')
  })

  test('S41: a Read-only database refuses to cancel or end anything', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page, { mode: /Read-only/ })
    await tab.getByTestId('sql-open-activity').click()
    await tab.getByRole('button', { name: 'End session 18342' }).click()
    await expect(tab.getByTestId('sql-activity').getByRole('alert')).toContainText('app-prod is read-only in Dalan')
    await expect(page.getByTestId('session-confirm')).toHaveCount(0)
  })
})

test.describe('Import, export and backup (S43, F26)', () => {
  test('S43: export writes the chosen format through Rust and reports the rows', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await tab.getByTestId('sql-open-transfer').click()
    const t = tab.getByTestId('sql-transfer')
    await t.getByLabel('Table', { exact: true }).selectOption('public.customers')
    await t.getByRole('button', { name: 'JSON' }).click()
    await t.getByRole('button', { name: 'Save as…' }).click()
    await expect(t.getByRole('region', { name: 'Export', exact: true }).locator('[data-part=done]')).toContainText('Saved /tmp/customers.json · 50 rows')
    expect((await fake.calls()).includes('export:customers.json')).toBe(true)
  })

  test('S43: import maps the file\'s columns, is reviewed as a COPY, and loads', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await tab.getByTestId('sql-open-transfer').click()
    const t = tab.getByTestId('sql-transfer')
    await t.getByLabel('Table', { exact: true }).selectOption('public.customers')
    await t.getByRole('button', { name: 'Choose file…' }).click()
    const map = t.getByRole('table', { name: 'Column mapping' })
    await expect(map).toContainText('who')
    // id and name matched by name; "who" and "note" are skipped until chosen.
    await expect(map.getByLabel('Column for id')).toHaveValue('id')
    await expect(map.getByLabel('Column for who').locator('option:checked')).toHaveText('skip')
    await map.getByLabel('Column for who').selectOption('name')
    await map.getByLabel('Column for name').selectOption({ label: 'skip' })
    await t.getByRole('button', { name: 'Review import' }).click()
    const d = page.getByTestId('import-confirm')
    await expect(d).toContainText('Load 3 rows into public.customers on app-prod?')
    await expect(d.getByLabel('Statement')).toHaveText('COPY "public"."customers" ("id", "name") FROM STDIN WITH (FORMAT csv)')
    await d.getByRole('button', { name: 'Load rows' }).click()
    await expect(t.getByRole('region', { name: 'Import', exact: true }).locator('[data-part=done]')).toContainText('Loaded 3 rows into public.customers.')
  })

  test('S43: a Read-only database refuses an import before any dialog', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page, { mode: /Read-only/ })
    await tab.getByTestId('sql-open-transfer').click()
    const t = tab.getByTestId('sql-transfer')
    await t.getByRole('button', { name: 'Choose file…' }).click()
    await t.getByRole('button', { name: 'Review import' }).click()
    await expect(t.getByRole('region', { name: 'Import', exact: true }).locator('[data-part=error]')).toContainText('app-prod is read-only in Dalan')
    await expect(page.getByTestId('import-confirm')).toHaveCount(0)
  })

  test('S43: backup shows the tool on the host and saves the dump', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openShop(page)
    await tab.getByTestId('sql-open-transfer').click()
    const b = tab.getByTestId('sql-transfer').getByRole('region', { name: 'Backup' })
    await expect(b.locator('[data-part=tool]')).toContainText('pg_dump (PostgreSQL) 16.4')
    await b.getByRole('button', { name: 'Back up…' }).click()
    await expect(b.locator('[data-part=done]')).toContainText('Saved /tmp/app-2026-11-24.dump · 5.0 MB')
    expect((await fake.calls()).includes('backup:app.dump')).toBe(true)
  })
})
