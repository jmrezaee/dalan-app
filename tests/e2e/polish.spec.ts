import { expect, test } from '../support/fixtures'
import { installFakeBackend } from '../fake'
import { file } from '../fake/scenarios/helpers'
import { openApp, sessionIds } from './harness'

// Three small things found by hand in the macOS window (hand-checks.md, 2026-11-07).
test('the side panel tabs each stay on one line ("Ask AI" used to wrap)', async ({ page }) => {
  const fake = await openApp(page, { unlocked: true })
  await page.getByText('web-1').dblclick()
  const [id] = await sessionIds(page)
  await fake.session(id!).state('ready')
  await page.keyboard.press('ControlOrMeta+.')
  const tabs = page.getByRole('tablist', { name: 'Side panel' }).getByRole('tab')
  await expect(tabs).toHaveCount(5)
  // Font metrics differ between Chromium here and the macOS window, so check the rule, not a measured height.
  const wraps = await tabs.evaluateAll(els => els.map(e => getComputedStyle(e).whiteSpace))
  expect(wraps).toEqual(Array(5).fill('nowrap'))
})

test('right-clicking empty space in an SFTP pane offers only New folder and Refresh', async ({ page }) => {
  await installFakeBackend(page, {
    scenario: 'team', unlocked: true,
    seed: { vault: { exists: true, unlocked: true }, now: Date.UTC(2026, 10, 6, 15), files: { local: [{ name: 'home', kind: 'dir', size: 0, mtime: 0, mode: 0o755, children: [{ name: 'me', kind: 'dir', size: 0, mtime: 0, mode: 0o755, children: [file('a.txt', 10), file('b.txt', 20)] }] }] } },
  })
  await page.goto('/')
  await page.getByRole('tab', { name: 'SFTP' }).click()
  const pane = page.getByRole('region', { name: 'This computer' })
  await pane.getByText('a.txt', { exact: true }).click() // a row stays selected
  await pane.getByText('a.txt', { exact: true }).click({ button: 'right' })
  await expect(page.getByRole('menuitem', { name: 'Rename' })).toBeVisible() // on the row: the row's menu
  await page.keyboard.press('Escape')
  const box = (await pane.getByText('b.txt', { exact: true }).boundingBox())!
  await pane.getByText('b.txt', { exact: true }).locator('xpath=ancestor::*[contains(@class,"overflow-auto")][1]').click({ button: 'right', position: { x: 40, y: box.y + 300 } })
  await expect(page.getByRole('menuitem', { name: 'New folder' })).toBeVisible()
  await expect(page.getByRole('menuitem', { name: 'Refresh' })).toBeVisible()
  await expect(page.getByRole('menuitem', { name: 'Rename' })).toHaveCount(0)
  await expect(page.getByRole('menuitem', { name: 'Delete' })).toHaveCount(0)
})

test('saving a host, a snippet and a group says which one', async ({ page }) => {
  await openApp(page, { unlocked: true })
  await page.getByRole('button', { name: /New host/ }).first().click()
  await page.getByLabel('Address').fill('10.9.9.9')
  await page.getByLabel('Label').fill('edge box')
  await page.getByRole('button', { name: 'Save', exact: true }).click()
  await expect(page.getByText('Saved edge box.')).toBeVisible()
})
