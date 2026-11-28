import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, fill, step } from './step'
import { openPg } from './dbhelpers'
import { unlock } from '../e2e/harness'

// Chapter 23 (replica/databases/restore-copy.md §9; E3, E4): two schemas compared with both DDLs, a sync that asks
// for the name, and two tables compared by key with the quick pass, then made to match.
chapter(23, { scenario: 'team' }, async (page) => {
  await unlock(page, MASTER)
  await openPg(page, 'stage-db')
  const tab = await openPg(page, 'app-prod')
  const cmp = tab.getByTestId('compare-tab')

  await step(page, { id: '23-01', flow: 'E3', screen: 'Compare', caption: 'Compare two schemas: the differences, and both DDLs side by side' }, async () => {
    await click(page, tab.getByTestId('sql-open-compare'))
    await click(page, cmp.getByTestId('compare-run'))
    await click(page, cmp.getByRole('list', { name: 'Differences' }).getByRole('button', { name: /Changed column id/ }))
    await expect(cmp.getByLabel('Source DDL')).toHaveText('bigint NOT NULL')
  })

  await step(page, { id: '23-02', flow: 'E3', screen: 'Compare', caption: 'Sync: one script, drops last; a type change asks for the name' }, async () => {
    await click(page, cmp.getByTestId('compare-sync'))
    const plan = cmp.getByTestId('compare-plan')
    await fill(page, plan.getByRole('textbox', { name: 'Type stage-db to confirm' }), 'stage-db')
    await click(page, plan.getByTestId('compare-commit'))
    await expect(cmp.locator('[data-part=report]')).toContainText('Ran 4 statements.')
  })

  await step(page, { id: '23-03', flow: 'E4', screen: 'Compare', caption: 'Data in a table: compared by key, with the quick pass' }, async () => {
    await click(page, cmp.getByRole('radiogroup', { name: 'Compare' }).getByText('Data in a table'))
    await fill(page, cmp.getByRole('textbox', { name: 'Source table: name' }), 'customers')
    await click(page, cmp.getByTestId('data-columns'))
    await click(page, cmp.getByTestId('data-compare'))
    await expect(cmp.locator('[data-part=data-counts]')).toContainText('Identical24,994')
  })

  await step(page, { id: '23-04', flow: 'E4', screen: 'Compare', caption: 'Make the target match: inserts, updates and deletes in one transaction' }, async () => {
    await click(page, cmp.getByTestId('data-match'))
    await click(page, cmp.getByTestId('data-match-commit'))
    await expect(cmp.locator('[data-part=matched]')).toContainText('Inserted 2, updated 3 and deleted 1 rows')
  })
})
