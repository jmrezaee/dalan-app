import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, fill, press, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 13 (F07, S21, S22): two panes, a typed path with `~` and `..`, hidden files, a new folder, rename, permissions,
// a drag with a conflict, a cancelled transfer, a folder with no access, and "Edit in default app".
chapter(13, { scenario: 'team' }, async (page, fake) => {
  await unlock(page, MASTER)
  const left = page.getByRole('region', { name: 'This computer' })
  const right = page.getByRole('region', { name: 'web-1' })
  const row = (pane: ReturnType<typeof page.getByRole>, name: string) => pane.getByRole('row').filter({ has: page.getByText(name, { exact: true }) }).first()

  await step(page, { id: '13-01', flow: 'F07', screen: 'S21', caption: 'Two panes: this computer on the left, a server on the right' }, async () => {
    await click(page, page.getByRole('tab', { name: 'SFTP' }))
    await click(page, page.getByRole('button', { name: 'This computer' }).or(page.getByRole('button', { name: 'Choose a place' })).first())
    await click(page, page.getByRole('menuitem', { name: 'This computer' }))
    await click(page, page.getByRole('button', { name: 'Choose a place' }))
    await click(page, page.getByRole('menuitem', { name: 'web-1', exact: true }))
    await expect(row(right, 'deploy.sh')).toBeVisible()
  })

  await step(page, { id: '13-02', flow: 'F07', screen: 'S21', caption: 'A typed path understands ~ and ..' }, async () => {
    await click(page, right.getByRole('button', { name: 'Type a path' }))
    await fill(page, right.getByRole('textbox', { name: 'Path' }), '~/../../srv')
    await press(page, 'Enter')
    await expect(row(right, 'index.html')).toBeVisible()
    await click(page, right.getByRole('button', { name: 'Type a path' }))
    await fill(page, right.getByRole('textbox', { name: 'Path' }), '~')
    await press(page, 'Enter')
    await expect(row(right, 'deploy.sh')).toBeVisible()
  })

  await step(page, { id: '13-03', flow: 'F07', screen: 'S21', caption: 'Hidden files, a new folder, and a rename with F2' }, async () => {
    await click(page, right.getByRole('button', { name: 'Show hidden files' }))
    await expect(row(right, '.bashrc')).toBeVisible()
    await click(page, right.getByRole('button', { name: 'New folder' }))
    await fill(page, page.getByRole('dialog').getByLabel('Name'), 'backups')
    await press(page, 'Enter')
    await expect(row(right, 'backups')).toBeVisible()
    await click(page, row(right, 'backups'))
    await press(page, 'F2')
    await fill(page, page.getByRole('dialog').getByLabel('Name'), 'archive')
    await press(page, 'Enter')
    await expect(row(right, 'archive')).toBeVisible()
  })

  await step(page, { id: '13-04', flow: 'F07', screen: 'S21', caption: 'Permissions on a file' }, async () => {
    await click(page, row(right, 'deploy.sh'), { button: 'right' })
    await click(page, page.getByRole('menuitem', { name: 'Permissions…' }))
    await expect(page.getByRole('dialog').getByText('Permissions')).toBeVisible()
    await page.getByRole('button', { name: 'Cancel' }).click()
  })

  await step(page, { id: '13-05', flow: 'F07', screen: 'S22', caption: 'Drag two files across: progress, then a conflict, then "Keep both"' }, async () => {
    await click(page, row(left, 'notes.txt'))
    await page.keyboard.down('Shift')
    await click(page, row(left, 'report.pdf'))
    await page.keyboard.up('Shift')
    await row(left, 'notes.txt').dragTo(right.getByRole('table').or(right).first())
    await expect(row(right, 'report.pdf')).toBeVisible()
    // The same files again: the name is taken.
    await row(left, 'notes.txt').dragTo(right.getByRole('table').or(right).first())
    await expect(page.getByRole('button', { name: 'Keep both' })).toBeVisible()
    await click(page, page.getByRole('checkbox', { name: 'Apply to all' }))
    await click(page, page.getByRole('button', { name: 'Keep both' }))
    await expect(right.getByText(/\(copy\)/).first()).toBeVisible()
  })

  await step(page, { id: '13-06', flow: 'F07', screen: 'S22', caption: 'Cancel a big transfer half way' }, async () => {
    await fake.manualTransfers(true)
    await row(left, 'report.pdf').dragTo(right.getByRole('table').or(right).first())
    await expect(page.getByRole('button', { name: 'Cancel transfer' }).first()).toBeVisible()
    await click(page, page.getByRole('button', { name: 'Cancel transfer' }).first())
    await fake.manualTransfers(false)
  })

  await step(page, { id: '13-07', flow: 'F07', screen: 'S21', caption: 'A folder you may not open says so' }, async () => {
    await click(page, right.getByRole('button', { name: 'Type a path' }))
    await fill(page, right.getByRole('textbox', { name: 'Path' }), '/private')
    await press(page, 'Enter')
    await expect(page.getByText('No access')).toBeVisible()
    await click(page, right.getByRole('button', { name: 'Go up' }))
  })

  await step(page, { id: '13-08', flow: 'F07', screen: 'S21', caption: 'Edit a file in your own app, then upload it; a script is refused' }, async () => {
    await click(page, right.getByRole('button', { name: 'Type a path' }))
    await fill(page, right.getByRole('textbox', { name: 'Path' }), '~')
    await press(page, 'Enter')
    await click(page, row(right, 'app.log'), { button: 'right' })
    await click(page, page.getByRole('menuitem', { name: 'Edit in default app' }))
    await fake.editSaved()
    await expect(page.getByText('app.log was saved.')).toBeVisible()
    await click(page, page.getByRole('button', { name: 'Upload' }))
    await expect(page.getByText('Uploaded app.log.')).toBeVisible()
    await click(page, row(right, 'deploy.sh'), { button: 'right' })
    await click(page, page.getByRole('menuitem', { name: 'Edit in default app' }))
    await expect(page.getByText(/would run on your computer/)).toBeVisible()
  })
})
