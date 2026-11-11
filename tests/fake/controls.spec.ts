import { expect, test } from '../support/fixtures'
import { installFakeBackend } from './index'
import { MASTER } from './commands/vault'

// The fake's own controls: what the walkthrough and the other specs build on.
const open = async (page: import('@playwright/test').Page) => {
  const fake = await installFakeBackend(page, { scenario: 'team', unlocked: false })
  await page.goto('/')
  await page.getByLabel('Master password').fill(MASTER)
  await page.getByRole('button', { name: 'Unlock' }).click()
  return fake
}
const termText = (page: import('@playwright/test').Page) => page.evaluate(() => {
  const term = Object.values((window as any).__dalanTerm ?? {})[0] as any
  const b = term?.buffer.active
  return b ? Array.from({ length: b.length }, (_, i) => b.getLine(i)?.translateToString(true)).join('\n') : ''
})

test('the team scenario fills the vault, and the clock is fixed', async ({ page }) => {
  await open(page)
  await expect(page.getByText('Production', { exact: true }).first()).toBeVisible()
  await expect(page.getByText('bastion', { exact: true }).first()).toBeVisible()
  expect(await page.evaluate(() => new Date().toISOString().slice(0, 10))).toBe('2026-11-06')
})

test('connect(): a known key goes straight to a live shell and a scripted server prints', async ({ page }) => {
  const fake = await open(page)
  await page.getByText('bastion', { exact: true }).first().dblclick()
  const [id] = await fake.sessionIds()
  expect(await fake.session(id!).connect()).toBe('ready')
  await fake.session(id!).runScript('ubuntu-login')
  await expect.poll(() => termText(page)).toContain('Welcome to Ubuntu 24.04.1 LTS')
})

test('rotateHostKey(): the next connect is blocked as a changed key, and Close refuses it', async ({ page }) => {
  const fake = await open(page)
  await fake.rotateHostKey('h-bastion')
  await page.getByText('bastion', { exact: true }).first().dblclick()
  const [id] = await fake.sessionIds()
  const result = fake.session(id!).connect()
  await expect(page.getByRole('heading', { name: 'This server\'s identity changed' })).toBeVisible()
  await page.getByRole('button', { name: 'Close', exact: true }).click()
  expect(await result).toBe('changed')
})

test('dropAll(): several dropped sessions raise one card', async ({ page }) => {
  const fake = await open(page)
  for (const name of ['bastion', 'roaming laptop']) {
    await page.getByRole('tab', { name: 'Vault' }).click()
    await page.getByText(name, { exact: true }).first().dblclick()
    const ids = await fake.sessionIds()
    await fake.session(ids[ids.length - 1]!).state('ready')
  }
  await fake.dropAll()
  await expect(page.getByText('2 connections dropped')).toBeVisible()
})

test('failNext(): the next call fails once with the error you chose', async ({ page }) => {
  const fake = await open(page)
  await fake.failNext('host_save', { code: 'validation', message: 'The port must be between 1 and 65535.' })
  await page.getByRole('button', { name: 'New host', exact: true }).click()
  await page.getByLabel('Address').fill('10.1.1.1')
  await page.getByRole('button', { name: /^(Save|Connect)/ }).first().click()
  await expect(page.getByText('The port must be between 1 and 65535.')).toBeVisible()
})

test('tunnel(): a state change reaches the Port forwarding list', async ({ page }) => {
  const fake = await open(page)
  await page.getByLabel('Vault', { exact: true }).getByText('Port forwarding').click()
  await expect(page.getByText('Staging web')).toBeVisible()
  await fake.tunnel('f-web').state('error', 'Port 8081 is already in use.')
  await expect(page.getByText('Port 8081 is already in use.')).toBeVisible()
})

test('the test build names the elements a role or label cannot (test IDs)', async ({ page }) => {
  const fake = await open(page)
  await expect(page.getByTestId('host-card-h-roam')).toBeVisible()
  await expect(page.getByTestId('group-card-g-prod')).toBeVisible()
  await expect(page.getByTestId('app-tabbar')).toBeVisible()
  await page.getByTestId('host-card-h-roam').dblclick()
  const [id] = await fake.sessionIds()
  await expect(page.getByTestId('status-overlay')).toBeVisible()
  const key = await page.evaluate(() => Object.keys((window as any).__dalanTerm ?? {})[0] ?? '')
  await expect(page.getByTestId(`terminal-${key}`)).toBeVisible()
  await fake.session(id!).state('ready')
})

test('Ctrl+Shift+C copies the selected terminal text', async ({ page }) => {
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write'])
  const fake = await open(page)
  await page.getByText('bastion', { exact: true }).first().dblclick()
  const [id] = await fake.sessionIds()
  await fake.session(id!).connect()
  await fake.session(id!).output('copy-me-please')
  await expect.poll(() => termText(page)).toContain('copy-me-please')
  await page.evaluate(() => (Object.values((window as any).__dalanTerm)[0] as any).selectAll())
  await page.locator('.xterm-helper-textarea').first().focus()
  await page.keyboard.press('Control+Shift+C')
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain('copy-me-please')
})
