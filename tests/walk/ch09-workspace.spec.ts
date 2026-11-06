import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, press, step } from './step'
import { unlock } from '../e2e/harness'
import { terminalText } from './helpers'

// Chapter 9 (F08, F09, S13, S19): run a snippet on three servers, a workspace opens, broadcast, panes, a template.
chapter(9, { scenario: 'team' }, async (page, fake) => {
  await unlock(page, MASTER)

  await step(page, { id: '09-01', flow: 'F08', screen: 'S13', caption: 'Run "Update packages" on its three servers' }, async () => {
    await click(page, page.getByLabel('Vault', { exact: true }).getByText('Snippets'))
    await click(page, page.getByRole('row').or(page.locator('div').filter({ hasText: /^Update packages/ })).filter({ hasText: 'Update packages' }).getByRole('button', { name: 'Run' }).first())
    await expect(page.getByRole('dialog').getByText('Update packages')).toBeVisible()
    await click(page, page.getByRole('button', { name: /^Run on 3 hosts/ }))
  })

  await step(page, { id: '09-02', flow: 'F09', screen: 'S19', caption: 'A workspace opens with one pane per server' }, async () => {
    await expect(page.getByRole('button', { name: /Broadcast/ })).toBeVisible()
    const ids = await fake.sessionIds()
    expect(ids).toHaveLength(3)
    await fake.session(ids[0]!).state('ready')
    await fake.session(ids[1]!).state('ready')
    await fake.session(ids[2]!).error('host_unreachable', 'db-1 is down')
    await fake.session(ids[0]!).runScript('apt-update')
    await expect.poll(() => terminalText(page)).toContain('12 packages can be upgraded')
  })

  await step(page, { id: '09-03', flow: 'F09', screen: 'S19', caption: 'Broadcast types into every live pane and skips the one that is down' }, async () => {
    await click(page, page.getByRole('button', { name: /Broadcast/ }))
    await expect(page.getByText('Broadcasting to 2')).toBeVisible()
  })

  await step(page, { id: '09-04', flow: 'F09', screen: 'S19', caption: 'Focus shows one pane at a time' }, async () => {
    await click(page, page.getByRole('radio', { name: 'Focus' }).or(page.getByRole('button', { name: 'Focus' })).first())
    await click(page, page.getByRole('radio', { name: 'Split' }).or(page.getByRole('button', { name: 'Split' })).first())
  })

  await step(page, { id: '09-05', flow: 'F09', screen: 'S19', caption: 'Save the layout as a template' }, async () => {
    await click(page, page.getByRole('button', { name: /Save as template/ }))
    await page.getByRole('dialog').getByRole('textbox').fill('Update run')
    await press(page, 'Enter')
    await expect(page.getByText('Update run').first()).toBeVisible()
  })
})
