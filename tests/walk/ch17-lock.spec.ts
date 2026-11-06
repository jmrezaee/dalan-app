import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, fill, press, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 17 (S24): lock the vault, five wrong passwords start a cooldown on the page clock, then unlock.
chapter(17, { scenario: 'team' }, async (page) => {
  await unlock(page, MASTER)
  const password = page.getByLabel('Master password')

  await step(page, { id: '17-01', screen: 'S24', caption: 'Lock the vault: the unlock screen comes back' }, async () => {
    await click(page, page.getByRole('button', { name: 'Lock', exact: true }))
    await expect(password).toBeVisible()
    await expect(page.getByRole('button', { name: 'Unlock' })).toBeVisible()
  })

  await step(page, { id: '17-02', screen: 'S24', caption: 'A wrong password is refused' }, async () => {
    await fill(page, password, 'not-it')
    await press(page, 'Enter')
    await expect(page.getByText('That password didn\'t work.')).toBeVisible()
  })

  await step(page, { id: '17-03', screen: 'S24', caption: 'Five wrong tries start a cooldown' }, async () => {
    for (let i = 0; i < 4; i++) {
      await fill(page, password, `wrong-${i}`)
      await press(page, 'Enter')
      await page.waitForTimeout(150)
    }
    await expect(page.getByRole('button', { name: /Try again in \d+s/ })).toBeVisible()
  })

  await step(page, { id: '17-04', screen: 'S24', caption: 'After the cooldown, the right password unlocks' }, async () => {
    await page.clock.runFor(31_000)
    await expect(page.getByRole('button', { name: 'Unlock' })).toBeEnabled()
    await fill(page, password, MASTER)
    await press(page, 'Enter')
    await expect(page.getByLabel('Vault', { exact: true })).toBeVisible()
  })
})
