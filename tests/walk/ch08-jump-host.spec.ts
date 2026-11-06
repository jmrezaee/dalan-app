import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, dblclick, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 8 (F05): a server behind a bastion. The route is shown, and each hop asks about its own key.
chapter(8, { scenario: 'team' }, async (page, fake) => {
  await unlock(page, MASTER)
  let id = ''

  await step(page, { id: '08-01', flow: 'F05', screen: 'S02', caption: 'web-1 lives in Production, whose route goes through the bastion' }, async () => {
    await click(page, page.getByText('Production', { exact: true }).first())
    await expect(page.getByText('web-1', { exact: true }).first()).toBeVisible()
  })

  await step(page, { id: '08-02', flow: 'F05', screen: 'S18', caption: 'Connecting shows the route: via bastion' }, async () => {
    await dblclick(page, page.getByRole('row').filter({ has: page.getByText('web-1', { exact: true }) }).first())
    id = (await fake.sessionIds())[0]!
    await fake.session(id).event({ type: 'hop', via: ['bastion'] })
    await expect(page.getByText('via bastion').first()).toBeVisible()
  })

  await step(page, { id: '08-03', flow: 'F05', screen: 'S18', caption: 'The first question is about the bastion\'s key' }, async () => {
    await fake.session(id).ask({ type: 'host_key_new', host: 'bastion', key_type: 'ssh-ed25519', fingerprint: 'SHA256:fake-bastion-hop' }, 'hop1')
    await expect(page.getByRole('heading', { name: 'First time connecting to bastion' })).toBeVisible()
    await click(page, page.getByRole('button', { name: 'Trust and connect' }))
  })

  await step(page, { id: '08-04', flow: 'F05', screen: 'S18', caption: 'Then web-1 asks about its own' }, async () => {
    await fake.session(id).ask({ type: 'host_key_new', host: 'web-1', key_type: 'ssh-ed25519', fingerprint: 'SHA256:fake-web-1-inner' }, 'hop2')
    await expect(page.getByRole('heading', { name: 'First time connecting to web-1' })).toBeVisible()
    await click(page, page.getByRole('button', { name: 'Trust and connect' }))
    await fake.session(id).state('ready')
    await expect(page.getByRole('heading', { name: /First time/ })).toHaveCount(0)
  })
})
