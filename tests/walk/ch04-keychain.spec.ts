import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, fill, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 4 (F02, S05 to S08): generate a key, import one by paste (a passphrase that is wrong, then right), an identity,
// a certificate, and install the key on web-2.
chapter(4, { scenario: 'team' }, async (page, fake) => {
  await unlock(page, MASTER)
  const keyCard = (name: string) => page.getByText(name, { exact: true }).first()

  await step(page, { id: '04-01', flow: 'F02', screen: 'S05', caption: 'The Keychain lists keys and identities' }, async () => {
    await click(page, page.getByLabel('Vault', { exact: true }).getByText('Keychain'))
    await expect(keyCard('work laptop')).toBeVisible()
    await expect(page.getByText('CERT', { exact: true }).first()).toBeVisible()
  })

  await step(page, { id: '04-02', flow: 'F02', screen: 'S06', caption: 'Generate an Ed25519 key' }, async () => {
    await click(page, page.getByRole('button', { name: 'Generate key', exact: true }))
    await fill(page, page.getByLabel('Label'), 'walk-ed25519')
    await click(page, page.getByRole('button', { name: 'Generate and save' }))
    await expect(keyCard('walk-ed25519')).toBeVisible()
  })

  await step(page, { id: '04-03', flow: 'F02', screen: 'S06', caption: 'Import a key by pasting it: the passphrase is asked for' }, async () => {
    await click(page, page.getByRole('button', { name: 'More create options' }))
    await click(page, page.getByRole('menuitem', { name: /Import key/ }))
    await fill(page, page.getByLabel('Label'), 'old-prod')
    await fill(page, page.getByLabel('Private key'), '-----BEGIN OPENSSH PRIVATE KEY-----\nENCRYPTED\n-----END OPENSSH PRIVATE KEY-----')
    await click(page, page.getByRole('button', { name: 'Import key' }))
    await expect(page.getByText('This key is protected by a passphrase.')).toBeVisible()
  })

  await step(page, { id: '04-04', flow: 'F02', screen: 'S06', caption: 'A wrong passphrase is refused, the right one imports' }, async () => {
    await fill(page, page.getByRole('dialog').getByLabel('Passphrase', { exact: true }), 'wrong')
    await click(page, page.getByRole('button', { name: 'Import key' }))
    await expect(page.getByRole('alert')).toBeVisible()
    await fill(page, page.getByRole('dialog').getByLabel('Passphrase', { exact: true }), 'right-passphrase')
    await click(page, page.getByRole('button', { name: 'Import key' }))
    await expect(keyCard('old-prod')).toBeVisible()
  })

  await step(page, { id: '04-05', flow: 'F02', screen: 'S07', caption: 'Make an identity: a username and a password together' }, async () => {
    await click(page, page.getByRole('button', { name: 'More create options' }))
    await click(page, page.getByRole('menuitem', { name: 'New identity' }))
    await fill(page, page.getByLabel('Label'), 'staging deploy')
    await fill(page, page.getByLabel('Username'), 'stage')
    await fill(page, page.getByLabel('Password', { exact: true }), 'a-test-password')
    await click(page, page.getByRole('button', { name: 'Save identity' }))
    await expect(page.getByText('staging deploy').first()).toBeVisible()
  })

  await step(page, { id: '04-06', flow: 'F02', screen: 'S05', caption: 'Attach a certificate to a key' }, async () => {
    await click(page, keyCard('walk-ed25519'), { button: 'right' })
    await click(page, page.getByRole('menuitem', { name: 'Attach certificate…' }))
    await fill(page, page.getByRole('dialog').getByLabel('Certificate', { exact: true }), 'ssh-ed25519-cert-v01@openssh.com AAAAWALKCERT')
    await click(page, page.getByRole('button', { name: 'Attach certificate' }))
    await expect(page.getByText('CERT', { exact: true })).toHaveCount(2)
  })

  await step(page, { id: '04-07', flow: 'F02', screen: 'S08', caption: 'Install the key on web-2' }, async () => {
    await click(page, keyCard('walk-ed25519'), { button: 'right' })
    await click(page, page.getByRole('menuitem', { name: 'Install on a server…' }))
    await click(page, page.getByRole('dialog').getByLabel('Server', { exact: true }))
    await click(page, page.getByRole('option', { name: /web-2/ }))
    await click(page, page.getByRole('button', { name: 'Install key' }))
    const ids = await fake.sessionIds()
    const id = ids[ids.length - 1]!
    await fake.session(id).state('connecting')
    await fake.session(id).state('authenticating')
    await fake.session(id).event({ type: 'exit', status: 0 })
    await expect(page.getByRole('button', { name: 'Done' })).toBeVisible()
  })
})
