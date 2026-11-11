import { expect, test } from '../support/fixtures'
import type { Locator } from '@playwright/test'
import type { Fake } from '../fake'
import { openApp } from './harness'

/** Sends the link until the page is listening (it registers its listener a moment after it mounts). */
const arrive = (fake: Fake, dialog: Locator, target: { user: string | null, host: string, port: number } | null) => expect(async () => {
  await fake.receiveLink(target)
  await expect(dialog).toBeVisible({ timeout: 500 })
}).toPass()

// ssh:// links (spec section 9.1): Rust parses them strictly (open_url.rs has the tests); the UI always asks first.
test.describe('ssh:// links', () => {
  test('a link asks first, with Cancel focused, and Cancel connects to nothing', async ({ page }) => {
    const fake = await openApp(page, { unlocked: true })
    const dialog = page.getByRole('alertdialog', { name: /Open a connection to/ })
    await arrive(fake, dialog, { user: 'deploy', host: '10.0.0.5', port: 22 })
    await expect(dialog).toContainText('deploy@10.0.0.5:22')
    await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeFocused()
    await page.keyboard.press('Escape')
    await expect(dialog).toHaveCount(0)
    const calls = await fake.calls()
    expect(calls.filter(c => /^(host_save|session_open|connect)/.test(c))).toEqual([])
  })

  test('Connect on an unknown address saves it and opens a session', async ({ page }) => {
    const fake = await openApp(page, { unlocked: true })
    const dialog = page.getByRole('alertdialog', { name: /Open a connection to/ })
    await arrive(fake, dialog, { user: 'deploy', host: '10.9.9.9', port: 2222 })
    await dialog.getByRole('button', { name: 'Connect' }).click()
    await expect(dialog).toHaveCount(0)
    await expect.poll(async () => (await fake.calls()).includes('host_save')).toBe(true)
    expect((await fake.sessionIds()).length).toBe(1)
  })

  test('a link to a saved server names it and opens it as it is', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    const dialog = page.getByRole('alertdialog', { name: /Open a connection to/ })
    // bastion is a saved server in the team scenario: take its address, user and port from the list
    const bastion = await page.evaluate(async () => {
      const h = (await (window as any).__TAURI_INTERNALS__.invoke('hosts_tree')).hosts.find((x: any) => x.label === 'bastion')
      return { user: h.effective_username, host: h.address, port: h.effective_port }
    })
    await arrive(fake, dialog, bastion)
    await expect(dialog).toContainText('bastion, one of your saved servers')
    await dialog.getByRole('button', { name: 'Connect' }).click()
    await expect.poll(async () => (await fake.sessionIds()).length).toBe(1)
    expect(await fake.calls()).not.toContain('host_save')
  })

  test('a link that is not a valid ssh:// address is a toast, not a question', async ({ page }) => {
    const fake = await openApp(page, { unlocked: true })
    await expect(async () => {
      await fake.receiveLink(null)
      await expect(page.getByText('This link isn\'t a valid ssh:// address.')).toBeVisible({ timeout: 500 })
    }).toPass()
    await expect(page.getByRole('alertdialog')).toHaveCount(0)
  })

  test('two links wait their turn', async ({ page }) => {
    const fake = await openApp(page, { unlocked: true })
    const dialog = page.getByRole('alertdialog', { name: /Open a connection to/ })
    await arrive(fake, dialog, { user: null, host: 'one.example.com', port: 22 })
    await fake.receiveLink({ user: null, host: 'two.example.com', port: 22 })
    await expect(dialog).toContainText('one.example.com')
    await page.keyboard.press('Escape')
    await expect(dialog).toContainText('two.example.com')
  })
})

// Settings > System shows on Windows only; the switches call the Rust commands that write the registry.
test.describe('Settings > System (Windows)', () => {
  test('is not there off Windows', async ({ page }) => {
    await openApp(page, { unlocked: true })
    await page.getByRole('button', { name: 'Settings' }).first().click()
    await expect(page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'About' })).toBeVisible()
    await expect(page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'System' })).toHaveCount(0)
  })

  test('the switches turn the two integrations on and off, and a pinned app is explained', async ({ page }) => {
    const fake = await openApp(page, { unlocked: true })
    await fake.setSystem({ edition: 'user', install_dir: 'C:\\Users\\me\\AppData\\Local\\Dalan', other_handler: 'putty.exe' })
    await page.getByRole('button', { name: 'Settings' }).first().click()
    await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'System' }).click()
    await expect(page.getByText('putty.exe currently opens these links')).toBeVisible()
    await page.getByRole('switch', { name: 'Open ssh:// links with Dalan' }).click()
    await expect(page.getByRole('switch', { name: 'Open ssh:// links with Dalan' })).toBeChecked()
    await page.getByRole('switch', { name: 'Start Dalan when I sign in to Windows' }).click()
    await expect(page.getByRole('switch', { name: 'Start Dalan when I sign in to Windows' })).toBeChecked()
    expect((await fake.calls()).filter(c => c === 'system_integration_set').length).toBe(2)

    await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'About' }).click()
    await expect(page.getByText('For you')).toBeVisible()
    await expect(page.getByText('C:\\Users\\me\\AppData\\Local\\Dalan')).toBeVisible()
  })
})
