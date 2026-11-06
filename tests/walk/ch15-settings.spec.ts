import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, dblclick, fill, hover, press, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 15 (S23): every section of Settings, a rebound shortcut that shows up in the labels, the bell, and the AI helper
// with no key, then a key, then a suggestion that is inserted (not run).
chapter(15, { scenario: 'team' }, async (page, fake) => {
  await unlock(page, MASTER)
  const section = (name: string) => page.getByRole('button', { name, exact: true }).or(page.getByRole('tab', { name, exact: true })).first()

  await step(page, { id: '15-01', screen: 'S23', caption: 'Settings has a page for each part of the app' }, async () => {
    await press(page, 'ControlOrMeta+,')
    for (const name of ['Appearance', 'Terminal', 'Logs', 'AI', 'Shortcuts', 'Security', 'About']) {
      await click(page, section(name))
      await expect(section(name)).toBeVisible()
    }
  })

  await step(page, { id: '15-02', screen: 'S23', caption: 'Rebind New tab to ⌘Y: the labels everywhere follow' }, async () => {
    await click(page, section('Shortcuts'))
    const row = page.locator('.row').filter({ hasText: /^New tab/ })
    await click(page, row.getByRole('button', { name: 'Change' }))
    await expect(page.getByText('Press keys…')).toBeVisible()
    await press(page, 'ControlOrMeta+y')
    await expect(row.locator('kbd')).toContainText(/Y/)
    await hover(page, page.getByRole('button', { name: 'New tab', exact: true }).first())
    await expect(page.getByRole('tooltip').getByText(/Y/).first()).toBeVisible()
  })

  await step(page, { id: '15-03', screen: 'S23', caption: 'Turn on the bell sound' }, async () => {
    await click(page, section('Terminal'))
    await click(page, page.getByRole('switch', { name: 'Bell sound' }))
    await expect(page.getByRole('switch', { name: 'Bell sound' })).toBeChecked()
  })

  await step(page, { id: '15-04', screen: 'S20', caption: 'Ask AI without a key says where to add one' }, async () => {
    await page.getByRole('tab', { name: 'Vault' }).click().catch(() => {})
    await click(page, page.getByLabel('Vault', { exact: true }).getByText('Hosts', { exact: true }))
    await dblclick(page, page.getByRole('row').filter({ has: page.getByText('bastion', { exact: true }) }).first())
    const id = (await fake.sessionIds())[0]!
    await fake.session(id).state('ready')
    await click(page, page.getByRole('button', { name: /^Snippets/ }))
    await click(page, page.getByRole('tab', { name: 'Ask AI', exact: true }))
    await fill(page, page.getByLabel('Describe what you want to do'), 'show how much disk is left')
    await click(page, page.getByRole('button', { name: 'Suggest a command' }))
    await expect(page.getByRole('alert')).toContainText('API key')
  })

  await step(page, { id: '15-05', screen: 'S23', caption: 'Add a key, ask again, and insert the suggestion without running it' }, async () => {
    await press(page, 'ControlOrMeta+,')
    await click(page, section('AI'))
    await fill(page, page.locator('input[type=password]').first(), 'sk-ant-test-key')
    await click(page, page.getByRole('button', { name: 'Save', exact: true }))
    await expect(page.getByRole('button', { name: 'Remove key' })).toBeVisible()
    await page.getByRole('tab', { name: /bastion/ }).click()
    await click(page, page.getByRole('button', { name: 'Suggest a command' }))
    await expect(page.getByText('df -h').first()).toBeVisible()
    await click(page, page.getByRole('button', { name: 'Insert' }))
    const written = await fake.written()
    expect(written.map(w => w.text).join('')).toContain('df -h')
    expect(written.map(w => w.text).join('')).not.toContain('\n')
  })
})
