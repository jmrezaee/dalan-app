import { expect, test } from '../support/fixtures'
import { installFakeBackend } from '../fake'
import { SCREENS } from '../a11y/screens'

// Visual regression (replica/testing/test-plan.md section 7): every screen state in dark and light, against baselines
// made in the pinned Linux Playwright image (scripts/in-playwright-docker.sh), so every machine draws the same pixels.
// The clock is fixed, animations are off and the data is the fake team vault, so the pictures only change when the UI does.
for (const theme of ['dark', 'light'] as const) {
  for (const s of SCREENS) {
    test(`${s.id}-${theme}`, async ({ page }) => {
      const fake = await installFakeBackend(page, { scenario: s.scenario, unlocked: s.unlocked ?? true, theme })
      await page.goto('/')
      if (s.unlocked === false) await expect(page.getByLabel('Master password')).toBeVisible()
      else await expect(page.getByLabel('Vault', { exact: true })).toBeVisible()
      await s.open(page, fake)
      await page.waitForTimeout(500) // transitions and the xterm first paint
      await expect(page).toHaveScreenshot(`${s.id}-${theme}.png`)
    })
  }
}
