import { expect, test } from '../support/fixtures'
import type { Locator } from '@playwright/test'
import type { Fake } from '../fake'
import { openApp } from './harness'

/** Sends the event until the page is listening (it registers its listener a moment after it mounts). */
const ask = (fake: Fake, dialog: Locator, n: number) => expect(async () => {
  await fake.quitRequested(n)
  await expect(dialog).toBeVisible({ timeout: 500 })
}).toPass()

// Closing the window or ⌘Q with live sessions asks first (found by hand on macOS: it quit without a word).
// Rust holds the quit back and sends `quit-requested`; the UI asks. The Rust decision has its own tests (quit.rs).
test.describe('quit with live sessions', () => {
  test('Keep working is the default, and Esc keeps working too', async ({ page }) => {
    const fake = await openApp(page, { unlocked: true })
    const dialog = page.getByRole('alertdialog', { name: 'Quit Dalan?' })
    await ask(fake, dialog, 2)
    await expect(dialog).toContainText('2 sessions are still open')
    await expect(dialog.getByRole('button', { name: 'Keep working' })).toBeFocused()
    await page.keyboard.press('Escape')
    await expect(dialog).toHaveCount(0)
    expect(await fake.calls()).toContain('quit_cancel')
    expect(await fake.calls()).not.toContain('quit_app')
  })

  test('Quit anyway ends the app', async ({ page }) => {
    const fake = await openApp(page, { unlocked: true })
    const dialog = page.getByRole('alertdialog', { name: 'Quit Dalan?' })
    await ask(fake, dialog, 1)
    await expect(dialog).toContainText('1 session is still open')
    await dialog.getByRole('button', { name: 'Quit anyway' }).click()
    await expect.poll(async () => (await fake.calls()).includes('quit_app')).toBe(true)
  })
})
