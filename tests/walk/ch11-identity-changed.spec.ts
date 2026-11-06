import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, dblclick, fill, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 11 (F12, S14): a server presents a different key. The connection is blocked, Close is the default,
// and replacing the saved key needs the server's name typed.
chapter(11, { scenario: 'team' }, async (page, fake) => {
  await unlock(page, MASTER)
  let id = ''
  let result: Promise<string>

  await step(page, { id: '11-01', flow: 'F12', screen: 'S14', caption: 'Known hosts lists what Dalan has trusted' }, async () => {
    await click(page, page.getByLabel('Vault', { exact: true }).getByText('Known hosts'))
    await expect(page.getByText('bastion.example.com').first()).toBeVisible()
  })

  await step(page, { id: '11-02', flow: 'F12', screen: 'S18', caption: 'The bastion is rekeyed: connecting is blocked' }, async () => {
    await fake.rotateHostKey('h-bastion', { type: 'ecdsa' })
    await click(page, page.getByLabel('Vault', { exact: true }).getByText('Hosts', { exact: true }))
    await dblclick(page, page.getByRole('row').filter({ has: page.getByText('bastion', { exact: true }) }).first())
    id = (await fake.sessionIds())[0]!
    result = fake.session(id).connect()
    await expect(page.getByRole('heading', { name: 'This server\'s identity changed' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Close', exact: true })).toBeFocused()
  })

  await step(page, { id: '11-03', flow: 'F12', screen: 'S18', caption: 'Replacing the saved key asks you to type the server\'s name' }, async () => {
    await click(page, page.getByRole('button', { name: 'Replace saved key…' }))
    const confirm = page.getByRole('button', { name: /Replace and connect/ })
    await expect(confirm).toBeDisabled()
    await fill(page, page.getByRole('textbox').last(), 'bastion')
    await expect(confirm).toBeEnabled()
    await click(page, confirm)
    expect(await result!).toBe('ready')
  })
})
