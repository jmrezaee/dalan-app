import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, fill, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 2 (F10, S25): import hosts from an SSH config: a filter, Select all, a warning, a duplicate left unticked.
chapter(2, { scenario: 'empty' }, async (page) => {
  await step(page, { id: '02-01', flow: 'F10', screen: 'S24', caption: 'Unlock the empty vault' }, async () => {
    await unlock(page, MASTER)
    await expect(page.getByLabel('Vault', { exact: true })).toBeVisible()
  })

  await step(page, { id: '02-02', flow: 'F10', screen: 'S25', caption: 'The empty vault offers Import; choose an SSH config' }, async () => {
    await click(page, page.getByRole('button', { name: 'Import', exact: true }))
    await click(page, page.getByRole('button', { name: 'Choose file…' }))
    await expect(page.getByText('2 of 3 selected')).toBeVisible()
  })

  await step(page, { id: '02-03', flow: 'F10', screen: 'S25', caption: 'Warnings are shown, and a duplicate starts unticked' }, async () => {
    await expect(page.getByText('An Include line was not followed.')).toBeVisible()
    await expect(page.getByText('ProxyCommand was ignored.')).toBeVisible()
    await expect(page.getByText('Already added')).toBeVisible()
    await expect(page.getByRole('checkbox', { name: 'Import web-1' })).toHaveAttribute('aria-checked', 'false')
  })

  await step(page, { id: '02-04', flow: 'F10', screen: 'S25', caption: 'Filter the list, then Select all acts on what is shown' }, async () => {
    const search = page.getByRole('dialog').getByLabel('Search', { exact: true })
    await fill(page, search, 'web')
    await click(page, page.getByRole('checkbox', { name: 'Select all' }))
    await expect(page.getByText('3 of 3 selected')).toBeVisible()
    await click(page, page.getByRole('checkbox', { name: 'Select all' }))
    await fill(page, search, '')
    await expect(page.getByText('2 of 3 selected')).toBeVisible()
  })

  await step(page, { id: '02-05', flow: 'F10', screen: 'S02', caption: 'Import the two ticked hosts and they appear' }, async () => {
    await click(page, page.getByRole('button', { name: 'Import 2 hosts' }))
    await expect(page.getByText('api', { exact: true }).first()).toBeVisible()
  })
})
