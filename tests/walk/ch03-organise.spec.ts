import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, fill, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 3 (F04, S02 to S04): a group with a username, hosts dragged in, "from Production" on a host, a nested group,
// a tag filter, list view and search.
chapter(3, { scenario: 'flat' }, async (page) => {
  await unlock(page, MASTER)
  const card = (name: string) => page.getByRole('row').filter({ has: page.getByText(name, { exact: true }) }).first()

  await step(page, { id: '03-01', flow: 'F04', screen: 'S02', caption: 'Hosts, with no groups yet' }, async () => {
    await expect(page.getByText('web-1', { exact: true }).first()).toBeVisible()
  })

  await step(page, { id: '03-02', flow: 'F04', screen: 'S04', caption: 'Create the group Production with the username deploy' }, async () => {
    await click(page, page.getByRole('button', { name: 'More create options' }))
    await click(page, page.getByRole('menuitem', { name: 'New group' }))
    await fill(page, page.getByLabel('Name', { exact: true }), 'Production')
    await fill(page, page.getByLabel('Username'), 'deploy')
    await click(page, page.getByRole('button', { name: 'Save' }))
    await expect(page.getByText('Production', { exact: true }).first()).toBeVisible()
  })

  await step(page, { id: '03-03', flow: 'F04', screen: 'S02', caption: 'Drag three hosts onto the group' }, async () => {
    const group = page.getByText('Production', { exact: true }).first()
    for (const name of ['web-1', 'web-2', 'db-1']) await card(name).dragTo(group)
    await click(page, group)
    await expect(page.getByText('web-2', { exact: true }).first()).toBeVisible()
  })

  await step(page, { id: '03-04', flow: 'F04', screen: 'S03', caption: 'Open a host: its username comes from Production' }, async () => {
    await click(page, card('web-1'), { button: 'right' })
    await click(page, page.getByRole('menuitem', { name: 'Edit', exact: true }))
    await expect(page.getByText(/from Production/i).first()).toBeVisible()
  })

  await step(page, { id: '03-05', flow: 'F04', screen: 'S02', caption: 'A tag filters the hosts' }, async () => {
    await page.keyboard.press('Escape')
    await click(page, page.getByRole('button', { name: 'prod' }).first())
    await expect(page.getByText('stage-app', { exact: true })).toHaveCount(0)
    await click(page, page.getByRole('button', { name: 'prod' }).first())
  })

  await step(page, { id: '03-06', flow: 'F04', screen: 'S02', caption: 'List view and search narrow the hosts' }, async () => {
    await click(page, page.getByRole('button', { name: 'List view' }))
    await fill(page, page.getByPlaceholder('Search hosts'), 'db')
    await expect(page.getByText('db-1', { exact: true }).first()).toBeVisible()
    await expect(page.getByText('web-2', { exact: true })).toHaveCount(0)
  })
})
