import type { Locator, Page } from '@playwright/test'
import { expect, test } from '../support/fixtures'
import { openApp, seen } from './harness'

// P31: every in-app drag runs on pointer events (the window's native file drop takes HTML5 drag events away).
// Each drag here is a real pointer press, moves and release; none uses HTML5 drag events.
async function drag(page: Page, from: Locator, to: Locator, at: { x: number, y: number } = { x: 0.5, y: 0.5 }) {
  const a = (await from.boundingBox())!
  const b = (await to.boundingBox())!
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2)
  await page.mouse.down()
  await page.mouse.move(a.x + a.width / 2 + 8, a.y + a.height / 2 + 2, { steps: 2 })
  await page.mouse.move(b.x + b.width * at.x, b.y + b.height * at.y, { steps: 6 })
  await page.mouse.up()
}
const nav = (page: Page) => page.getByLabel('Vault', { exact: true })
const tabNames = (page: Page) => page.getByRole('tab').allInnerTexts().then(t => t.map(s => s.trim().split('\n').pop()!.trim()))

test.describe('in-app drags on pointer events (P31)', () => {
  test('tabs: a tab dropped on the left quarter of another goes before it; Escape cancels a drag', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    for (const h of ['bastion', 'roaming laptop']) {
      await page.getByRole('tab', { name: 'Vault' }).click()
      await page.getByRole('row').filter({ has: page.getByText(h, { exact: true }) }).first().dblclick()
      await seen(page.getByRole('tab', { name: new RegExp(`^${h}`) }))
    }
    expect((await tabNames(page)).slice(-2)).toEqual(['bastion', 'roaming laptop'])
    // Escape while dragging: nothing moves.
    const laptop = page.getByRole('tab', { name: /^roaming laptop/ }).locator('..')
    const bastion = page.getByRole('tab', { name: /^bastion/ }).locator('..')
    const a = (await laptop.boundingBox())!
    const b = (await bastion.boundingBox())!
    await page.mouse.move(a.x + 20, a.y + 10)
    await page.mouse.down()
    await page.mouse.move(b.x + 10, b.y + 10, { steps: 5 })
    await expect(page.locator('html[data-dragging=tab]')).toHaveCount(1)
    await page.keyboard.press('Escape')
    await page.mouse.up()
    expect((await tabNames(page)).slice(-2)).toEqual(['bastion', 'roaming laptop'])
    // A real drop before bastion.
    await drag(page, laptop, bastion, { x: 0.1, y: 0.5 })
    expect((await tabNames(page)).slice(-2)).toEqual(['roaming laptop', 'bastion'])
    // A plain click still selects (the drag threshold keeps clicks clicks).
    await page.getByRole('tab', { name: /^bastion/ }).click()
    await expect(page.getByRole('tab', { name: /^bastion/ })).toHaveAttribute('aria-selected', 'true')
  })

  test('hosts: a host card dropped on a group moves into it', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    const card = page.getByRole('row').filter({ has: page.getByText('bastion', { exact: true }) }).first()
    const group = page.getByText('Home lab', { exact: true }).first()
    await drag(page, card, group)
    await expect.poll(async () => (await fake.calls()).some(c => c.startsWith('hosts_move'))).toBe(true)
    await group.click()
    await seen(page.getByText('bastion', { exact: true }))
  })

  test('snippets: a snippet dropped on a package moves into it', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    await nav(page).getByText('Snippets', { exact: true }).click()
    const deploy = page.locator('section').filter({ has: page.getByRole('heading', { name: 'Deploy' }) })
    await drag(page, page.getByText('Who is on', { exact: true }), deploy.getByRole('heading', { name: 'Deploy' }))
    await expect.poll(async () => (await fake.calls()).includes('snippets_move')).toBe(true)
    await expect(deploy.getByText('Who is on', { exact: true })).toBeVisible()
  })

  test('SFTP: rows dragged from one pane to the other are copied there; not onto their own pane', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    const left = page.getByRole('region', { name: 'This computer' })
    const right = page.getByRole('region', { name: 'web-1' })
    const row = (pane: Locator, name: string) => pane.getByRole('row').filter({ has: page.getByText(name, { exact: true }) }).first()
    await page.getByRole('tab', { name: 'SFTP' }).click()
    await page.getByRole('button', { name: 'This computer' }).or(page.getByRole('button', { name: 'Choose a place' })).first().click()
    await page.getByRole('menuitem', { name: 'This computer' }).click()
    await page.getByRole('button', { name: 'Choose a place' }).click()
    await page.getByRole('menuitem', { name: 'web-1', exact: true }).click()
    await seen(row(right, 'deploy.sh'))
    // Onto its own pane: nothing is sent.
    await drag(page, row(left, 'notes.txt'), left.getByRole('grid'))
    expect((await fake.calls()).some(c => c.startsWith('sftp_copy') || c.startsWith('transfer'))).toBe(false)
    await drag(page, row(left, 'notes.txt'), right.getByRole('grid'))
    await seen(row(right, 'notes.txt'))
  })
})
