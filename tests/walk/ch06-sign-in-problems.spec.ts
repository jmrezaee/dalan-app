import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, fill, dblclick, press, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 6 (F01, S18): a wrong password, "Try another way" and a two-step code, an unreachable server, a failed key exchange.
chapter(6, { scenario: 'team' }, async (page, fake) => {
  await unlock(page, MASTER)
  const open = async (name: string) => {
    await page.getByRole('tab', { name: 'Vault' }).click()
    await dblclick(page, page.getByRole('row').filter({ has: page.getByText(name, { exact: true }) }).first())
    const ids = await fake.sessionIds()
    return ids[ids.length - 1]!
  }
  let id = ''

  await step(page, { id: '06-01', flow: 'F01', screen: 'S18', caption: 'The server asks for a password' }, async () => {
    id = await open('bastion')
    await fake.session(id).ask({ type: 'password', host: 'bastion', user: 'ops' }, 'pw1')
    await expect(page.getByRole('heading', { name: 'Password for bastion' })).toBeVisible()
  })

  await step(page, { id: '06-02', flow: 'F01', screen: 'S18', caption: 'A wrong password: sign-in failed, with other ways to try' }, async () => {
    await fill(page, page.locator('input[type=password]').last(), 'not-it')
    await press(page, 'Enter')
    await fake.session(id).event({ type: 'step', text: 'The server accepts: password, keyboard-interactive.' })
    await fake.session(id).event({ type: 'auth_methods', methods: ['password', 'keyboard_interactive'] })
    await fake.session(id).error('auth_failed', 'bastion rejected the password.')
    await expect(page.getByRole('heading', { name: 'Sign-in failed' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Verification code' })).toBeVisible()
  })

  await step(page, { id: '06-03', flow: 'F01', screen: 'S18', caption: 'Try another way: the server asks for a code' }, async () => {
    await click(page, page.getByRole('button', { name: 'Verification code' }))
    expect(await fake.calls(`reconnect:${id}:keyboard_interactive`)).toHaveLength(1)
    await fake.session(id).ask({ type: 'keyboard_interactive', host: 'bastion', name: 'Two-step sign-in', instructions: '', prompt: 'Verification code:', echo: true }, 'otp1')
    await expect(page.getByRole('heading', { name: 'Two-step sign-in' })).toBeVisible()
    await expect(page.getByText('Verification code:')).toBeVisible()
  })

  await step(page, { id: '06-04', flow: 'F01', screen: 'S18', caption: 'An unreachable server shows what Dalan tried' }, async () => {
    const other = await open('roaming laptop')
    await fake.session(other).event({ type: 'step', text: 'Connecting to laptop.example.com:22.' })
    await fake.session(other).error('host_unreachable', 'Can\'t reach laptop.example.com:22: timed out')
    await expect(page.getByRole('heading', { name: /Can't reach/ })).toBeVisible()
    await expect(page.getByText('Connecting to laptop.example.com:22.')).toBeVisible()
  })

  await step(page, { id: '06-05', flow: 'F01', screen: 'S18', caption: 'A key exchange that fails says so' }, async () => {
    const other = await open('bastion')
    await fake.session(other).error('kex_failed', 'Couldn\'t agree on encryption with bastion.example.com: no matching key exchange method.')
    await expect(page.getByText(/Couldn't agree on encryption/)).toBeVisible()
  })
})
