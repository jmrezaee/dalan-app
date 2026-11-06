import { expect, test } from '../support/fixtures'
import { openApp, seen } from './harness'

// S11 and S12: the snippet list, the editor, and dragging a snippet onto a package.
test.describe('snippets (S11, S12)', () => {
  test('S12: a new snippet is saved into the list, with its script', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    await page.getByLabel('Vault', { exact: true }).getByText('Snippets', { exact: true }).click()
    await page.getByRole('button', { name: 'New snippet', exact: true }).click()
    await page.locator('#sn-label').fill('Restart nginx')
    await page.locator('#sn-script').fill('sudo systemctl restart nginx')
    await page.getByRole('button', { name: /^Save/ }).click()
    await seen(page.getByText('Restart nginx', { exact: true }))
    await seen(page.getByText('sudo systemctl restart nginx'))
  })

  test('S11: dragging a snippet onto a package moves it (snippets_move)', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    await page.getByLabel('Vault', { exact: true }).getByText('Snippets', { exact: true }).click()
    await seen(page.getByText('Who is on', { exact: true }))
    const deploy = page.locator('section').filter({ has: page.getByRole('heading', { name: 'Deploy' }) })
    await page.locator('[draggable=true]').filter({ hasText: 'Who is on' }).dragTo(deploy)
    await expect.poll(async () => (await fake.calls()).includes('snippets_move')).toBe(true)
    await expect(deploy.getByText('Who is on', { exact: true })).toBeVisible()
  })
})
