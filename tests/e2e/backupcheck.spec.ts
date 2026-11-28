import { expect, test } from '../support/fixtures'
import type { Page } from '@playwright/test'
import { openApp } from './harness'

// D17 (replica/databases/restore-copy.md §9, E7): every backup is read back; the backup check restores it into a
// scratch database and counts rows against the source. The fake stands in for both.
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

test.describe('Backup check (E7)', () => {
  test('a backup is read back; the check restores it on a non-production server and compares rows', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    await addDb(page, 'scratch-box')
    const tab = await addDb(page, 'app-prod', { production: true })
    await tab.getByTestId('sql-open-transfer').click()
    const b = tab.getByTestId('sql-transfer').getByRole('region', { name: 'Backup' })
    await b.getByRole('button', { name: 'Back up…' }).click()
    await expect(b.locator('[data-part=read-check]')).toHaveText('Read back: PostgreSQL custom archive (gzip), 14 objects.')
    await b.getByTestId('backup-check').click()
    const dlg = page.getByTestId('backup-check-dialog')
    await expect(dlg.getByRole('radio', { name: 'The backup just made' })).toBeChecked()
    // app-prod is production: only the other server is offered.
    await expect(dlg.getByRole('combobox', { name: 'Restore it on' }).locator('option')).toHaveText(['scratch-box'])
    await dlg.getByTestId('backup-check-run').click()
    await expect(dlg.locator('[data-part=verdict]')).toContainText('Restored on scratch-box: some counts differ')
    await expect(dlg.getByRole('table', { name: 'Rows per table' })).toContainText('public.orders1,2001,201')
    expect(await fake.calls('backup-verify')).toContain('backup-verify:backup-file-1')

    // A backup in a folder on a host.
    await fake.putRemoteFile('h-bastion', '/home/me/backups/app-2026-11-20.dump')
    await dlg.getByRole('radio', { name: /A file in a folder on a host/ }).check()
    await dlg.getByTestId('backup-check-run').click()
    const picker = page.getByTestId('host-picker')
    await picker.getByRole('combobox', { name: 'Host' }).selectOption({ label: 'bastion' })
    await picker.getByRole('button', { name: 'Connect' }).click()
    await picker.getByRole('button', { name: 'backups' }).click()
    await picker.getByRole('button', { name: 'app-2026-11-20.dump' }).click()
    await picker.getByTestId('host-picker-use').click()
    await expect(picker).toHaveCount(0)
    await expect.poll(() => fake.calls('backup-verify')).toContainEqual(expect.stringMatching(/^backup-verify:sftp-\d+:\/home\/me\/backups\/app-2026-11-20\.dump$/))
  })

  test('a backup that doesn\'t read back says so', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await addDb(page, 'app-prod')
    await fake.restoreNext('cut')
    await tab.getByTestId('sql-open-transfer').click()
    const b = tab.getByTestId('sql-transfer').getByRole('region', { name: 'Backup' })
    await b.getByRole('button', { name: 'Back up…' }).click()
    await expect(b.getByRole('alert').filter({ hasText: 'doesn\'t read back' })).toContainText('It ends early or is damaged')
  })
})
