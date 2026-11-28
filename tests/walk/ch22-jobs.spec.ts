import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, fill, step } from './step'
import { openPg } from './dbhelpers'
import { unlock } from '../e2e/harness'

// Chapter 22 (replica/databases/restore-copy.md §9; E1, E2, E8): a backup saved as a job into a folder on a host,
// scheduled with retention, its missed runs and a failure, and the dot on Jobs.
chapter(22, { scenario: 'team' }, async (page, fake) => {
  await unlock(page, MASTER)
  const tab = await openPg(page, 'app-prod')
  const nav = page.getByRole('navigation', { name: 'Vault' })

  await step(page, { id: '22-01', flow: 'E8', screen: 'S43', caption: 'Save a backup as a job, into a folder on a saved host (picked over SFTP)' }, async () => {
    await click(page, tab.getByTestId('sql-open-transfer'))
    const b = tab.getByTestId('sql-transfer').getByRole('region', { name: 'Backup' })
    await click(page, b.getByRole('button', { name: 'Save as a job…' }))
    await fill(page, b.getByRole('textbox', { name: 'Job name' }), 'Nightly')
    await click(page, b.getByTestId('backup-save-job-host'))
    const picker = page.getByTestId('host-picker')
    await picker.getByRole('combobox', { name: 'Host' }).selectOption({ label: 'bastion' })
    await click(page, picker.getByRole('button', { name: 'Connect' }))
    await click(page, picker.getByTestId('host-picker-use'))
    await expect(b).toContainText('Saved as the job “Nightly”')
  })

  await step(page, { id: '22-02', flow: 'E2', screen: 'Jobs', caption: 'Jobs: schedule it every day at 03:30 and keep the last 7' }, async () => {
    await click(page, page.getByRole('tab', { name: 'Vault' }))
    await click(page, nav.getByRole('link', { name: /Jobs/ }))
    const row = page.getByTestId('job-Nightly')
    await click(page, row.getByTestId('job-schedule'))
    const dlg = page.getByTestId('schedule-dialog')
    await click(page, dlg.getByRole('radio', { name: 'Every day' }))
    await fill(page, dlg.getByLabel('Time'), '03:30')
    await fill(page, dlg.getByRole('spinbutton', { name: 'Number of backups to keep' }), '7')
    await click(page, dlg.getByTestId('schedule-save'))
    await expect(row.locator('[data-part=schedule]')).toContainText('Every day at 03:30')
  })

  await step(page, { id: '22-03', flow: 'E2', screen: 'Jobs', caption: 'After an unlock: three runs were missed while locked, and it ran once' }, async () => {
    await fake.scheduledRun('Nightly', 'ok', 3)
    await expect(page.getByTestId('job-Nightly').locator('[data-part=missed]')).toContainText('Missed 3 runs')
  })

  await step(page, { id: '22-04', flow: 'E2', screen: 'Jobs', caption: 'A failed run: a dot on Jobs until it is opened, and the reason in History' }, async () => {
    await click(page, nav.getByRole('link', { name: /Hosts/ }))
    await fake.scheduledRun('Nightly', 'failed')
    await expect(page.getByRole('img', { name: 'A scheduled job failed' })).toBeVisible()
    await click(page, nav.getByRole('link', { name: /Jobs/ }))
    await click(page, page.getByTestId('job-Nightly').getByRole('button', { name: 'History' }))
    await expect(page.getByTestId('job-Nightly')).toContainText('failed')
  })
})
