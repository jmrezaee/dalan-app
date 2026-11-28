import type { Page } from '@playwright/test'
import { expect } from '@playwright/test'
import { click, dblclick, fill } from './step'

/** Adds a PostgreSQL database to the vault (shown, like a person would) and opens it in a tab. */
export async function openPg(page: Page, name: string, opts: { production?: boolean } = {}) {
  const nav = page.getByRole('navigation', { name: 'Vault' })
  if (await page.getByRole('tab', { name: 'Vault' }).isVisible()) await click(page, page.getByRole('tab', { name: 'Vault' }))
  await click(page, nav.getByRole('link', { name: /Databases/ }))
  await click(page, page.getByRole('button', { name: 'New database' }))
  await click(page, page.getByTestId('new-database-dialog').getByRole('button', { name: 'Continue' }))
  const s = page.getByTestId('db-form-sheet')
  await fill(page, s.getByLabel('Name', { exact: true }), name)
  if (opts.production) await click(page, s.getByRole('radio', { name: 'Production' }))
  await fill(page, s.getByLabel('Address'), `${name}.internal`)
  await fill(page, s.getByLabel('Database', { exact: true }), 'app')
  await fill(page, s.getByLabel('Username'), 'app')
  await click(page, s.getByRole('button', { name: 'Save' }))
  await expect(s).toHaveCount(0)
  await dblclick(page, page.getByRole('row', { name: new RegExp(name) }))
  const tab = page.getByTestId(`sql-tab-${name}`)
  await expect(tab.getByRole('tree', { name: 'Schemas and tables' })).toBeVisible()
  return tab
}
