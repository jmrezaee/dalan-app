import { expect, test } from '../support/fixtures'
import { openApp, seen } from './harness'

// F04: groups and inheritance, beyond what the walkthrough shows.
test.describe('F04 groups and inheritance', () => {
  test('F04: a host shows what it takes from its group, an own value overrides it, and clearing it goes back', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    await page.getByText('Production', { exact: true }).first().click()
    await page.getByRole('row').filter({ has: page.getByText('web-2', { exact: true }) }).first().click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Edit', exact: true }).click()
    const user = page.getByLabel('Username')
    await seen(page.getByText('from Production').first())
    const hints = await page.getByText('from Production').count()
    await user.fill('someone-else')
    await expect(page.getByText('from Production')).toHaveCount(hints - 1) // an own username wins, the rest is still inherited
    await user.fill('')
    await expect(page.getByText('from Production')).toHaveCount(hints) // back to inherited
  })

  test('F04: deleting a group moves its hosts up one level instead of deleting them', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    await page.getByText('Production', { exact: true }).first().click()
    const europe = page.locator('[aria-label=Groups] > *').filter({ hasText: 'Europe' }).first()
    await europe.getByRole('button', { name: 'Group actions' }).click()
    await page.getByRole('menuitem', { name: 'Delete' }).click()
    await page.getByRole('button', { name: 'Delete group' }).click()
    await expect(page.getByText('db-1', { exact: true }).first()).toBeVisible() // now directly in Production
    const model = await fake.model()
    expect(model.hosts.find((h: { id: string }) => h.id === 'h-db1').group_id).toBe('g-prod')
    expect(model.groups.some((g: { id: string }) => g.id === 'g-prod-eu')).toBe(false)
  })
})
