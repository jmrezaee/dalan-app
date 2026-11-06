import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, dblclick, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 10 (F11): every connection drops at once, one card, Reconnect all, automatic reconnect on the page clock,
// and a shell that ended with `exit` stays closed.
chapter(10, { scenario: 'team' }, async (page, fake) => {
  await unlock(page, MASTER)
  const open = async (name: string) => {
    await page.getByRole('tab', { name: 'Vault' }).click()
    await dblclick(page, page.getByRole('row').filter({ has: page.getByText(name, { exact: true }) }).first())
    const ids = await fake.sessionIds()
    return ids[ids.length - 1]!
  }
  let a = ''
  let b = ''

  await step(page, { id: '10-01', flow: 'F11', screen: 'S18', caption: 'Two live connections' }, async () => {
    a = await open('bastion')
    await fake.session(a).state('ready')
    b = await open('roaming laptop')
    await fake.session(b).state('ready')
    await expect(page.getByLabel('roaming laptop: connected')).toBeVisible()
  })

  await step(page, { id: '10-02', flow: 'F11', screen: 'S18', caption: 'The network drops: one card for both connections' }, async () => {
    await fake.dropAll()
    await expect(page.getByText('2 connections dropped')).toBeVisible()
  })

  await step(page, { id: '10-03', flow: 'F11', screen: 'S18', caption: 'Reconnect all' }, async () => {
    await click(page, page.getByRole('button', { name: 'Reconnect all' }))
    await expect.poll(async () => (await fake.calls()).filter(c => c.startsWith('reconnect:')).length).toBe(2)
    await fake.session(a).state('ready')
    await fake.session(b).state('ready')
  })

  await step(page, { id: '10-04', flow: 'F11', screen: 'S23', caption: 'Turn on automatic reconnect, drop again, and the tabs come back by themselves' }, async () => {
    await page.keyboard.press('ControlOrMeta+,')
    await click(page, page.getByRole('button', { name: 'Terminal', exact: true }).or(page.getByRole('tab', { name: 'Terminal', exact: true })).first())
    await click(page, page.getByRole('switch', { name: 'Reconnect automatically' }))
    await page.getByRole('tab', { name: /bastion/ }).click()
    const before = (await fake.calls()).filter(c => c.startsWith('reconnect:')).length
    await fake.dropAll()
    await page.clock.runFor(2500) // the first retry is after 2 seconds
    await expect.poll(async () => (await fake.calls()).filter(c => c.startsWith('reconnect:')).length).toBeGreaterThan(before)
  })

  await step(page, { id: '10-05', flow: 'F11', screen: 'S18', caption: '`exit` ends a shell for good: it does not come back' }, async () => {
    await fake.session(a).state('ready')
    await fake.session(b).state('ready')
    const before = (await fake.calls()).filter(c => c.startsWith('reconnect:')).length
    await fake.session(a).event({ type: 'exit', status: 0 })
    await fake.session(a).state('disconnected')
    await page.clock.runFor(60_000)
    expect((await fake.calls()).filter(c => c.startsWith('reconnect:')).length).toBe(before)
    await expect(page.getByText(/connections dropped/)).toHaveCount(0)
  })
})
