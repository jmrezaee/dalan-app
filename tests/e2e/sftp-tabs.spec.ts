import { expect, test } from '../support/fixtures'
import { openApp } from './harness'

// P30: more than one SFTP tab. Each tab has its own two panes and its own transfers.

test('a host can open in a new SFTP tab, and the tabs keep their own servers', async ({ page }) => {
  await openApp(page, { unlocked: true })

  // The first SFTP tab: web-1 in its right pane.
  await page.getByText('web-1').click({ button: 'right' })
  await page.getByRole('menuitem', { name: 'Connect with SFTP', exact: true }).click()
  await expect(page.getByRole('region', { name: 'web-1' })).toBeVisible()

  // A second one for db-1.
  await page.getByRole('tab', { name: 'Vault' }).click()
  await page.getByText('db-1').click({ button: 'right' })
  await page.getByRole('menuitem', { name: 'Connect with SFTP in a new tab' }).click()
  await expect(page.getByRole('tab', { name: 'SFTP 2' })).toHaveAttribute('aria-selected', 'true')
  await expect(page.getByRole('region', { name: 'db-1' })).toBeVisible()
  await expect(page.getByRole('region', { name: 'web-1' })).toHaveCount(0) // that one is in the other tab

  // Back in the first tab nothing changed.
  await page.getByRole('tab', { name: 'SFTP', exact: true }).click()
  await expect(page.getByRole('region', { name: 'web-1' })).toBeVisible()
  await expect(page.getByRole('region', { name: 'db-1' })).toHaveCount(0)

  // ⌘1 Vault, ⌘2 SFTP, ⌘3 SFTP 2.
  await page.keyboard.press('ControlOrMeta+3')
  await expect(page.getByRole('region', { name: 'db-1' })).toBeVisible()

  // Closing the extra tab drops its connection and returns to the first one. The first has no close button.
  await page.getByRole('button', { name: 'Close SFTP 2' }).click()
  await expect(page.getByRole('tab', { name: 'SFTP 2' })).toHaveCount(0)
  await expect(page.getByRole('region', { name: 'web-1' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Close SFTP' })).toHaveCount(0)
  expect(await page.evaluate(() => (window as any).__fake.state.calls as string[])).toContainEqual(expect.stringMatching(/^sftp_close/))
})

test('the palette opens an empty SFTP tab', async ({ page }) => {
  await openApp(page, { unlocked: true })
  await expect(page.getByText('web-1').first()).toBeVisible()
  await page.keyboard.press('ControlOrMeta+k')
  await page.getByPlaceholder('Search hosts, tabs and actions').fill('new sftp tab')
  await page.keyboard.press('Enter')
  await expect(page.getByRole('tab', { name: 'SFTP 2' })).toHaveAttribute('aria-selected', 'true')
  await expect(page.getByRole('region', { name: 'This computer' })).toBeVisible() // the left pane starts on this computer
})
