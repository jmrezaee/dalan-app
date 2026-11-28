import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, fill, step } from './step'
import { openPg } from './dbhelpers'
import { unlock } from '../e2e/harness'

// Chapter 24 (replica/databases/restore-copy.md §9; E5, E6, E7): many tables out with masking, a folder of files
// back in (one into a new table), and a backup read back and checked by restoring it elsewhere.
chapter(24, { scenario: 'team' }, async (page) => {
  await unlock(page, MASTER)
  await openPg(page, 'scratch-box')
  const tab = await openPg(page, 'app-prod', { production: true })

  await step(page, { id: '24-01', flow: 'E6', screen: 'S43', caption: 'Export many tables into a zip, emails masked, with a manifest' }, async () => {
    await click(page, tab.getByTestId('sql-open-transfer'))
    const ex = tab.getByTestId('bulk-export')
    await click(page, ex.getByRole('checkbox', { name: 'public.customers' }))
    await click(page, ex.getByRole('checkbox', { name: 'public.orders' }))
    await click(page, ex.locator('summary'))
    await click(page, ex.getByRole('button', { name: 'Add a rule' }))
    await fill(page, ex.getByRole('textbox', { name: 'Masked column 1 of the bulk export' }), 'email')
    await click(page, ex.getByRole('radio', { name: 'A .zip file' }))
    await click(page, ex.getByTestId('bulk-export-run'))
    await expect(ex.locator('[data-part=bulk-done]')).toContainText('manifest.json')
  })

  await step(page, { id: '24-02', flow: 'E6', screen: 'S43', caption: 'Import a folder: each file to its table, a new one with a reviewed CREATE TABLE' }, async () => {
    const im = tab.getByTestId('bulk-import')
    await click(page, im.getByRole('button', { name: 'Choose a folder…' }))
    await click(page, im.getByTestId('bulk-file-loyalty.csv').getByRole('button', { name: 'CREATE TABLE' }))
    await click(page, im.getByTestId('bulk-import-review'))
    await click(page, im.getByTestId('bulk-import-commit'))
    await expect(im.getByRole('table', { name: 'Import report' })).toContainText('(new)')
  })

  await step(page, { id: '24-03', flow: 'E7', screen: 'S43', caption: 'Every backup is read back; "Check a backup…" restores it elsewhere and counts rows' }, async () => {
    const b = tab.getByTestId('sql-transfer').getByRole('region', { name: 'Backup' })
    await click(page, b.getByRole('button', { name: 'Back up…' }))
    await expect(b.locator('[data-part=read-check]')).toContainText('Read back')
    await click(page, b.getByTestId('backup-check'))
    await click(page, page.getByTestId('backup-check-run'))
    await expect(page.getByTestId('backup-check-dialog').getByRole('table', { name: 'Rows per table' })).toBeVisible()
  })
})
