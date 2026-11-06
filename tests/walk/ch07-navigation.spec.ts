import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, dblclick, fill, press, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 7 (F03, S01, S16, S17): New tab, quick connect, the command palette, tab dots, rename, close others.
chapter(7, { scenario: 'team' }, async (page, fake) => {
  await unlock(page, MASTER)
  const tab = (name: string | RegExp) => page.getByRole('tab', { name })

  await step(page, { id: '07-01', flow: 'F03', screen: 'S16', caption: 'New tab: recent servers and a search box' }, async () => {
    await press(page, 'ControlOrMeta+t')
    await expect(page.getByLabel('Search hosts').last()).toBeVisible()
    await expect(page.getByRole('region', { name: 'Recent' }).or(page.locator('section[aria-label=Recent]'))).toBeVisible()
  })

  await step(page, { id: '07-02', flow: 'F03', screen: 'S16', caption: 'Type user@host:port to connect to a server you never saved' }, async () => {
    await fill(page, page.getByLabel('Search hosts').last(), 'test@10.0.0.9:2222')
    await press(page, 'Enter')
    await expect(page.getByRole('heading', { name: 'Connecting to 10.0.0.9…' })).toBeVisible()
  })

  await step(page, { id: '07-03', flow: 'F03', screen: 'S17', caption: 'The command palette finds a server and opens it' }, async () => {
    await press(page, 'ControlOrMeta+k')
    await fill(page, page.getByPlaceholder('Search hosts, tabs and actions'), 'bastion')
    await press(page, 'ArrowDown')
    await press(page, 'Enter')
    await expect(tab(/bastion/)).toBeVisible()
  })

  await step(page, { id: '07-04', flow: 'F03', screen: 'S01', caption: 'Tab dots: yellow waits for you, green is new output, red is a failure' }, async () => {
    const ids = await fake.sessionIds()
    const [quick, bastion] = [ids[0]!, ids[1]!]
    await fake.session(bastion).state('ready')
    await fake.session(bastion).ask({ type: 'password', host: 'bastion', user: 'ops' }, 'sudo1')
    await expect(page.getByLabel('bastion: waiting for input')).toBeVisible()
    await fake.session(quick).state('ready')
    await fake.session(quick).output('background output\r\n')
    await expect(page.getByLabel('10.0.0.9: new output')).toBeVisible()
    await fake.session(quick).error('host_unreachable', 'lost')
    await expect(page.getByLabel('10.0.0.9: connection failed')).toBeVisible()
  })

  await step(page, { id: '07-05', flow: 'F03', screen: 'S01', caption: 'Rename a tab by double-clicking it' }, async () => {
    await dblclick(page, tab(/bastion/))
    await press(page, 'ControlOrMeta+a')
    await page.keyboard.type('prod bastion')
    await press(page, 'Enter')
    await expect(tab(/prod bastion/)).toBeVisible()
  })

  await step(page, { id: '07-06', flow: 'F03', screen: 'S01', caption: 'Close the other tabs from the tab menu' }, async () => {
    await click(page, tab(/prod bastion/), { button: 'right' })
    await click(page, page.getByRole('menuitem', { name: 'Close others', exact: true }))
    await expect(tab(/10\.0\.0\.9/)).toHaveCount(0)
    await expect(tab(/prod bastion/)).toBeVisible()
  })
})
