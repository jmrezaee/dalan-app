import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, fill, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 12 (F06, S09, S10): the three-step wizard, a port that is in use, a tunnel that becomes active, and the other states.
chapter(12, { scenario: 'team' }, async (page, fake) => {
  await unlock(page, MASTER)

  await step(page, { id: '12-01', flow: 'F06', screen: 'S09', caption: 'Tunnels in four states: active, reconnecting, error, stopped' }, async () => {
    await click(page, page.getByLabel('Vault', { exact: true }).getByText('Port forwarding'))
    await expect(page.getByLabel('Postgres: Active · 2')).toBeVisible()
    await expect(page.getByLabel(/SOCKS proxy: Reconnecting/)).toBeVisible()
    await expect(page.getByLabel(/Demo site: /)).toBeVisible()
    await expect(page.getByLabel('Staging web: Stopped')).toBeVisible()
  })

  await step(page, { id: '12-02', flow: 'F06', screen: 'S10', caption: 'The wizard asks for the type first' }, async () => {
    await click(page, page.getByRole('button', { name: 'New rule' }).or(page.getByRole('button', { name: /New (rule|tunnel)/ })).first())
    await expect(page.getByRole('radiogroup', { name: 'Type' })).toBeVisible()
    await click(page, page.getByRole('radio', { name: /Local/ }))
    await click(page, page.getByRole('button', { name: 'Next' }))
  })

  await step(page, { id: '12-03', flow: 'F06', screen: 'S10', caption: 'Port 5432 is taken: starting the rule says so' }, async () => {
    await fill(page, page.getByLabel('Label'), 'Walk database')
    await click(page, page.getByRole('dialog').getByLabel('Through server', { exact: true }))
    await click(page, page.getByRole('option', { name: /web-1/ }))
    await fill(page, page.getByRole('dialog').getByLabel('Port').first(), '5432')
    await fill(page, page.getByLabel(/^To \(as the server sees it\)/), '127.0.0.1')
    await fill(page, page.getByRole('dialog').getByLabel('Port').last(), '5432')
    await click(page, page.getByRole('button', { name: 'Next' }))
    await click(page, page.getByRole('button', { name: 'Save' }))
    await expect(page.getByLabel('Walk database: Stopped')).toBeVisible()
    await click(page, page.getByLabel('Walk database: Stopped').getByRole('button', { name: 'Start tunnel' }).or(page.getByRole('button', { name: 'Start tunnel' }).last()))
    await expect(page.getByText('Port 5432 is already in use. Try 15432.')).toBeVisible()
  })

  await step(page, { id: '12-04', flow: 'F06', screen: 'S10', caption: 'Use 15432 instead and start it' }, async () => {
    await click(page, page.getByRole('button', { name: 'Edit Walk database' }))
    await click(page, page.getByRole('button', { name: 'Skip the wizard' }).or(page.getByRole('dialog').getByLabel('Port').first()))
    await fill(page, page.getByRole('dialog').getByLabel('Port').first(), '15432')
    await click(page, page.getByRole('button', { name: 'Save' }))
    await click(page, page.getByRole('button', { name: 'Start tunnel' }).last())
    const id = (await fake.calls()).filter(c => c.startsWith('forward-start:')).pop()!.split(':')[1]!
    await fake.tunnel(id).state('active')
    await expect(page.getByLabel(/Walk database: Active/)).toBeVisible()
  })
})
