import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, dblclick, fill, press, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 19 (replica/databases/spec.md; S34 to S43, F20 to F26): a PostgreSQL database from the list to a tab, a table
// and its rows, a query that runs at once, a change that is shown first, the structure, the plan and the sessions.
chapter(19, { scenario: 'team' }, async (page) => {
  await unlock(page, MASTER)
  const nav = page.getByRole('navigation', { name: 'Vault' })
  const tab = page.getByTestId('sql-tab-app-prod')

  await step(page, { id: '19-01', flow: 'F20', screen: 'S35', caption: 'Add a PostgreSQL database: its address and sign-in, saved in the vault' }, async () => {
    await click(page, nav.getByRole('link', { name: /Databases/ }))
    await click(page, page.getByRole('button', { name: 'New database' }))
    await click(page, page.getByTestId('new-database-dialog').getByRole('radio', { name: /PostgreSQL/ }))
    await click(page, page.getByTestId('new-database-dialog').getByRole('button', { name: 'Continue' }))
    const s = page.getByTestId('db-form-sheet')
    await fill(page, s.getByLabel('Name', { exact: true }), 'app-prod')
    await fill(page, s.getByLabel('Address'), 'db.internal')
    await click(page, s.getByRole('button', { name: 'Save' }))
    await expect(page.getByRole('row', { name: /app-prod/ })).toBeVisible()
  })

  await step(page, { id: '19-02', flow: 'F21', screen: 'S36', caption: 'Open it: the tables on the left, the first page of a table on the right' }, async () => {
    await dblclick(page, page.getByRole('row', { name: /app-prod/ }))
    await click(page, tab.getByTestId('sql-object-orders'))
    await expect(tab.getByRole('grid', { name: 'public.orders rows' })).toBeVisible()
  })

  await step(page, { id: '19-03', flow: 'F21', screen: 'S37', caption: 'A query runs at once and shows its rows' }, async () => {
    await click(page, tab.getByRole('button', { name: 'New query' }))
    await click(page, tab.getByRole('textbox', { name: /SQL$/ }))
    await page.keyboard.insertText('select * from customers')
    await click(page, tab.getByTestId('sql-run'))
    await expect(tab.getByRole('button', { name: /Result 1 · 50 rows/ })).toBeVisible()
  })

  await step(page, { id: '19-04', flow: 'F22', screen: 'S42', caption: 'A change is held: Dalan says how many rows it reaches and asks first' }, async () => {
    await click(page, tab.getByRole('textbox', { name: /SQL$/ }))
    await press(page, 'ControlOrMeta+a')
    await page.keyboard.insertText('update customers set name = \'x\' where id = 1')
    await click(page, tab.getByTestId('sql-run'))
    await expect(page.getByTestId('sql-confirm')).toContainText('This will change 1 row.')
    await click(page, page.getByTestId('sql-confirm').getByRole('button', { name: 'Cancel' }))
  })

  await step(page, { id: '19-05', flow: 'F23', screen: 'S38', caption: 'The structure of a table: columns, keys and the statements behind it' }, async () => {
    await click(page, tab.getByTestId('sql-object-orders'))
    await click(page, tab.getByRole('group', { name: 'View' }).getByRole('button', { name: 'Structure' }))
    await expect(tab.getByTestId('structure-col-id')).toContainText('pk')
  })

  await step(page, { id: '19-06', flow: 'F24', screen: 'S41', caption: 'Who is connected: sessions, with a question before one is ended' }, async () => {
    await click(page, tab.getByTestId('sql-open-activity'))
    await expect(tab.getByTestId('session-18342')).toBeVisible()
    await click(page, tab.getByRole('button', { name: 'End session 18290' }))
    await expect(page.getByTestId('session-confirm')).toBeVisible()
    await click(page, page.getByTestId('session-confirm').getByRole('button', { name: 'Cancel' }))
  })

  await step(page, { id: '19-07', flow: 'F26', screen: 'S43', caption: 'Import, export and backup of the database' }, async () => {
    await click(page, tab.getByTestId('sql-open-transfer'))
    await expect(tab.getByTestId('sql-transfer')).toBeVisible()
    await expect(tab.getByTestId('sql-transfer').locator('[data-part="tool"]')).toContainText('pg_dump')
  })
})
