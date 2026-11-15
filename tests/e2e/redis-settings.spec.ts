import { expect, test } from '../support/fixtures'
import { openApp } from './harness'

// R7 of Redis: Settings > Databases (S23). What a new database starts with, how much one scan asks for, and
// whether console history is kept. Rust reads "keep console history" from the same saved value.
test('Settings > Databases: the defaults apply to a new database, the scan size to the key list, and history can be turned off', async ({ page }) => {
  const fake = await openApp(page, { scenario: 'team', unlocked: true })
  await page.getByRole('button', { name: 'Settings' }).first().click()
  await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Databases' }).click()
  await expect(page.getByRole('radio', { name: 'Confirm writes' })).toHaveAttribute('aria-checked', 'true')
  await expect(page.getByLabel('Key separator')).toHaveValue(':')
  await expect(page.getByLabel('Keys per scan')).toHaveValue('500')
  await expect(page.getByRole('switch', { name: 'Keep console history' })).toBeChecked()

  await page.getByRole('radio', { name: 'Read-only' }).click()
  await page.getByLabel('Key separator').fill('/')
  await page.getByLabel('Key separator').press('Tab')
  await page.getByLabel('Keys per scan').fill('200')
  await page.getByLabel('Keys per scan').press('Tab')
  await page.getByRole('switch', { name: 'Keep console history' }).click()
  await expect.poll(async () => JSON.parse((await fake.settings()).redis ?? '{}')).toEqual({ default_mode: 'read_only', key_separator: '/', scan_count: 200, keep_history: false })

  // A new database starts with them.
  await page.getByRole('tab', { name: 'Vault' }).click()
  await page.getByRole('navigation', { name: 'Vault' }).getByRole('link', { name: /Databases/ }).click()
  await page.getByRole('button', { name: 'New database' }).click()
  const sheet = page.getByRole('dialog', { name: /database/i })
  await expect(sheet.getByRole('radio', { name: /Read-only/ })).toHaveAttribute('aria-checked', 'true')
  await page.keyboard.press('Escape')

  // The key list asks for 200 keys a step, and the console keeps nothing.
  await page.getByRole('row', { name: /local/ }).dblclick()
  const tab = page.getByTestId('redis-tab-local')
  await expect(tab.getByText('Scanned 200 of 1,237 keys')).toBeVisible()
  await tab.getByRole('tab', { name: 'Console' }).click()
  await tab.getByRole('textbox', { name: 'Command' }).fill('PING')
  await page.keyboard.press('Enter')
  await expect(tab.getByTestId('console-entry').last()).toContainText('PONG')
  expect(await fake.redisHistory('r-local')).toEqual([])
})
