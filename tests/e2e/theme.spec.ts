import { expect, test } from '../support/fixtures'
import { installFakeBackend } from '../fake'

// The two themes (replica/design): dark is the default, light follows the saved choice, and both use the token colours.
test('dark is the default theme @theme', async ({ page }) => {
  await installFakeBackend(page, { scenario: 'team', unlocked: true })
  await page.goto('/')
  await expect(page.getByLabel('Vault', { exact: true })).toBeVisible()
  expect(await page.evaluate(() => document.documentElement.classList.contains('dark'))).toBe(true)
})

test('light is used when it is the saved choice, and the page background is the light token @theme', async ({ page }) => {
  await installFakeBackend(page, { scenario: 'team', unlocked: true, theme: 'light' })
  await page.goto('/')
  await expect(page.getByLabel('Vault', { exact: true })).toBeVisible()
  const { dark, bg, token } = await page.evaluate(() => ({
    dark: document.documentElement.classList.contains('dark'),
    bg: getComputedStyle(document.body).backgroundColor,
    token: (() => { const d = document.createElement('div'); d.style.background = 'var(--bg)'; document.body.append(d); const c = getComputedStyle(d).backgroundColor; d.remove(); return c })(),
  }))
  expect(dark).toBe(false)
  expect(bg).toBe(token)
})
