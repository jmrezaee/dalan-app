import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, fill, step } from './step'
import { openPg } from './dbhelpers'
import { unlock } from '../e2e/harness'

// Chapter 21 (replica/databases/restore-copy.md; S48, F31 and S49, F32): any dump restored into a new database after
// a look inside it, and a copy between two databases with masking, reviewed and run.
chapter(21, { scenario: 'team' }, async (page) => {
  await unlock(page, MASTER)
  await openPg(page, 'stage-db')
  const tab = await openPg(page, 'app-prod')

  await step(page, { id: '21-01', flow: 'F31', screen: 'S48', caption: 'Restore a dump: Dalan reads the file first and says what is inside' }, async () => {
    await click(page, tab.getByTestId('sql-open-transfer'))
    await click(page, tab.getByTestId('sql-restore'))
    const dlg = page.getByTestId('restore-dialog')
    await click(page, dlg.getByTestId('restore-pick-file'))
    await expect(dlg.getByRole('table', { name: 'Objects in the dump' })).toBeVisible()
  })

  await step(page, { id: '21-02', flow: 'F31', screen: 'S48', caption: 'Into a new database on the same server; the review names the target twice' }, async () => {
    const dlg = page.getByTestId('restore-dialog')
    await click(page, dlg.getByRole('button', { name: 'Next' }))
    await click(page, dlg.getByRole('radio', { name: 'A new database on the same server' }))
    await fill(page, dlg.getByTestId('restore-new-name'), 'restored')
    await click(page, dlg.getByTestId('restore-next'))
    await click(page, dlg.getByTestId('restore-confirm'))
    await expect(dlg.getByTestId('restore-result')).toContainText('Restored into restored')
    await click(page, dlg.getByRole('button', { name: 'Close' }).first())
  })

  await step(page, { id: '21-03', flow: 'F32', screen: 'S49', caption: 'Copy to another database: choose the tables and the target' }, async () => {
    await click(page, tab.getByTestId('sql-open-copy'))
    const copy = tab.getByTestId('copy-tab')
    await click(page, copy.getByRole('checkbox', { name: 'Copy public.customers' }))
    await click(page, copy.getByRole('checkbox', { name: 'Into a new database on that server' }))
    await fill(page, copy.getByRole('textbox', { name: 'New database name' }), 'scratch')
    await click(page, copy.getByTestId('copy-review'))
    await expect(copy.getByTestId('copy-plan')).toBeVisible()
  })

  await step(page, { id: '21-04', flow: 'F32', screen: 'S49', caption: 'Personal columns are suggested for masking; then the copy and its report' }, async () => {
    const copy = tab.getByTestId('copy-tab')
    await click(page, copy.getByTestId('copy-mask-suggested'))
    await click(page, copy.getByTestId('copy-run'))
    await expect(copy.getByTestId('copy-report')).toContainText('Copied into scratch')
  })
})
