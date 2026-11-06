import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, dblclick, press, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 16 (S02, S03, S18, S19, S21): switch to the light theme and look at the main screens again.
chapter(16, { scenario: 'team' }, async (page, fake) => {
  await unlock(page, MASTER)
  const isLight = () => page.evaluate(() => document.documentElement.classList.contains('light') || !document.documentElement.classList.contains('dark'))

  await step(page, { id: '16-01', screen: 'S23', caption: 'Settings, Appearance: choose Light' }, async () => {
    await press(page, 'ControlOrMeta+,')
    await click(page, page.getByRole('button', { name: 'Appearance', exact: true }).or(page.getByRole('tab', { name: 'Appearance', exact: true })).first())
    await click(page, page.getByRole('radio', { name: 'Light' }).or(page.getByRole('button', { name: 'Light', exact: true })).first())
    await expect.poll(isLight).toBe(true)
  })

  await step(page, { id: '16-02', screen: 'S02', caption: 'Hosts in light' }, async () => {
    await click(page, page.getByLabel('Vault', { exact: true }).getByText('Hosts', { exact: true }))
    await expect(page.getByText('Production', { exact: true }).first()).toBeVisible()
  })

  await step(page, { id: '16-03', screen: 'S03', caption: 'The host editor in light' }, async () => {
    await click(page, page.getByRole('row').filter({ has: page.getByText('bastion', { exact: true }) }).first(), { button: 'right' })
    await click(page, page.getByRole('menuitem', { name: 'Edit', exact: true }))
    await expect(page.getByLabel('Address')).toBeVisible()
    await press(page, 'Escape')
  })

  await step(page, { id: '16-04', screen: 'S18', caption: 'A live terminal in light' }, async () => {
    await dblclick(page, page.getByRole('row').filter({ has: page.getByText('bastion', { exact: true }) }).first())
    const id = (await fake.sessionIds())[0]!
    await fake.session(id).connect()
    await fake.session(id).runScript('nginx-status')
    await expect(page.locator('.xterm').first()).toBeVisible()
  })

  await step(page, { id: '16-05', screen: 'S19', caption: 'A workspace in light' }, async () => {
    await page.getByRole('tab', { name: 'Vault' }).click()
    await click(page, page.getByLabel('Vault', { exact: true }).getByText('Snippets'))
    await click(page, page.getByRole('row').or(page.locator('div').filter({ hasText: /^Update packages/ })).filter({ hasText: 'Update packages' }).getByRole('button', { name: 'Run' }).first())
    await click(page, page.getByRole('button', { name: /^Run on 3 hosts/ }))
    await expect(page.getByRole('button', { name: /Broadcast/ })).toBeVisible()
  })

  await step(page, { id: '16-06', screen: 'S21', caption: 'SFTP in light' }, async () => {
    await click(page, page.getByRole('tab', { name: 'SFTP' }))
    await expect(page.getByRole('region', { name: 'This computer' })).toBeVisible()
  })
})
