import { expect, test } from '../support/fixtures'
import { installFakeBackend } from '../fake'
import { file } from '../fake/scenarios/helpers'

// F06 and F07 cases the walkthrough does not reach.
test('F06: Enter on a rule toggles it, and each card says its state', async ({ page }) => {
  const fake = await installFakeBackend(page, { scenario: 'team', unlocked: true })
  await page.goto('/')
  await page.getByLabel('Vault', { exact: true }).getByText('Port forwarding', { exact: true }).click()
  const staging = page.getByLabel('Staging web: Stopped')
  await expect(staging).toBeVisible()
  await page.getByRole('button', { name: 'Start tunnel' }).last().focus()
  await page.keyboard.press('Enter')
  await expect.poll(async () => (await fake.calls()).some(c => c.startsWith('forward-start:'))).toBe(true)
  await fake.tunnel('f-web').state('reconnecting')
  await expect(page.getByLabel(/Staging web: Reconnecting/)).toBeVisible()
  await fake.tunnel('f-web').state('error', 'The server refused the forward.')
  await expect(page.getByText('The server refused the forward.')).toBeVisible()
})

test('F07: a folder with 600 files scrolls (the list is virtual) and sorting still works', async ({ page }) => {
  const many = Array.from({ length: 600 }, (_, i) => file(`file-${String(i).padStart(3, '0')}.log`, 100 + i))
  await installFakeBackend(page, {
    scenario: 'team', unlocked: true,
    seed: { vault: { exists: true, unlocked: true }, now: Date.UTC(2026, 10, 6, 15), files: { local: [{ name: 'home', kind: 'dir', size: 0, mtime: 0, mode: 0o755, children: [{ name: 'me', kind: 'dir', size: 0, mtime: 0, mode: 0o755, children: many }] }] } },
  })
  await page.goto('/')
  await page.getByRole('tab', { name: 'SFTP' }).click()
  const pane = page.getByRole('region', { name: 'This computer' })
  await expect(pane.getByText('file-000.log', { exact: true })).toBeVisible()
  expect(await pane.getByRole('row').count()).toBeLessThan(200) // only the visible rows are in the page
  await pane.getByRole('button', { name: /^Size/ }).click()
  await pane.getByRole('button', { name: /^Size/ }).click() // largest first
  await expect(pane.getByText('file-599.log', { exact: true })).toBeVisible()
  await pane.locator('div.overflow-auto, [class*=overflow-y]').first().evaluate(el => { el.scrollTop = el.scrollHeight })
  await expect(pane.getByText('file-000.log', { exact: true })).toBeVisible()
})
