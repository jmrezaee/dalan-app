import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, fill, step } from './step'

// Chapter 1 (F10, S24): a fresh install, a weak then a strong password, the meter, the mismatch message, and the empty vault.
chapter(1, { scenario: 'fresh' }, async (page) => {
  const password = page.getByLabel('Master password')
  const confirm = page.getByLabel('Confirm password')
  const create = page.getByRole('button', { name: /^Create/ })

  await step(page, { id: '01-01', flow: 'F10', screen: 'S24', caption: 'A new install asks for a master password' }, async () => {
    await expect(password).toBeVisible()
    await expect(confirm).toBeVisible()
  })

  await step(page, { id: '01-02', flow: 'F10', screen: 'S24', caption: 'A short password shows a weak meter' }, async () => {
    await fill(page, password, 'abc')
    await expect(page.getByLabel('Password strength: 1 of 5')).toBeVisible()
  })

  await step(page, { id: '01-03', flow: 'F10', screen: 'S24', caption: 'A strong passphrase fills the meter' }, async () => {
    await fill(page, password, MASTER + '-Plum7!')
    await expect(page.getByLabel(/Password strength: [45] of 5/)).toBeVisible()
  })

  await step(page, { id: '01-04', flow: 'F10', screen: 'S24', caption: 'Passwords that differ are refused' }, async () => {
    await fill(page, confirm, 'something else')
    await click(page, create)
    await expect(page.getByText('Passwords don\'t match.')).toBeVisible()
  })

  await step(page, { id: '01-05', flow: 'F10', screen: 'S24', caption: 'Fixing the confirmation clears the message' }, async () => {
    await fill(page, confirm, MASTER + '-Plum7!')
    await expect(page.getByText('Passwords don\'t match.')).toHaveCount(0)
  })

  await step(page, { id: '01-06', flow: 'F10', screen: 'S02', caption: 'The vault opens, empty' }, async () => {
    await click(page, create)
    await expect(page.getByLabel('Vault', { exact: true })).toBeVisible()
    await expect(page.getByText('web-1')).toHaveCount(0)
  })
})
