import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { Browser } from 'playwright'
import type { Server } from 'node:http'
import { enabled, focused, fakeEvent, gone, hasAttr, seen, fakeOutput, launchBrowser, openApp, serveApp, sessionIds, unlock } from './harness'

let browser: Browser
let server: Server
let url: string

beforeAll(async () => {
  ;({ server, url } = await serveApp())
  browser = await launchBrowser()
})
afterAll(async () => {
  await browser?.close()
  server?.close()
})

describe('unlock and the connection overlay (F01, F11, F12)', () => {
  it('a wrong master password is refused, the right one opens the vault', async () => {
    const page = await openApp(browser, url)
    await unlock(page, 'not-the-password')
    await seen(page.getByText('That password didn\'t work.').first())
    await unlock(page)
    await seen(page.getByText('web-1'))
    await page.close()
  })

  it('F01: connecting, trust the key, then a live shell shows output', async () => {
    const page = await openApp(browser, url, { unlocked: true })
    await page.getByText('web-1').dblclick()
    await seen(page.getByRole('heading', { name: 'Connecting to web-1…' }))
    const [id] = await sessionIds(page)
    await fakeEvent(page, id!, { type: 'prompt', prompt_id: 'p1', kind: { type: 'host_key_new', host: 'web-1', key_type: 'ssh-ed25519', fingerprint: 'SHA256:abc' } })
    await seen(page.getByRole('heading', { name: 'First time connecting to web-1' }))
    await seen(page.getByText('SHA256:abc'))
    await page.getByRole('button', { name: 'Trust and connect' }).click()
    const answered = await page.evaluate(() => (window as any).__fake.state.responses)
    expect(answered).toMatchObject([{ prompt: 'p1', answer: { accept: true, remember: true } }])
    await fakeEvent(page, id!, { type: 'state', state: 'ready' })
    await fakeOutput(page, id!, 'hello from the fake server\r\n')
    await gone(page.getByRole('alertdialog'))
    // xterm paints to a canvas, so the proof that it received the bytes is that it acknowledged them.
    await expect.poll(() => page.evaluate(() => (window as any).__fake.state.calls.some((c: string) => c.startsWith('ack:')))).toBe(true)
    await page.close()
  })

  it('F11: a dropped connection offers Reconnect, and Enter does it', async () => {
    const page = await openApp(browser, url, { unlocked: true })
    await page.getByText('web-1').dblclick()
    const [id] = await sessionIds(page)
    await fakeEvent(page, id!, { type: 'state', state: 'ready' })
    await fakeEvent(page, id!, { type: 'state', state: 'disconnected' })
    await seen(page.getByRole('heading', { name: 'Connection closed' }))
    await focused(page.getByRole('button', { name: 'Reconnect' }))
    await page.keyboard.press('Enter')
    const calls = await page.evaluate(() => (window as any).__fake.state.calls as string[])
    expect(calls).toContain(`reconnect:${id}`)
    await page.close()
  })

  it('F01: a failed sign-in shows the step log, offers another way, and Connect once answers without saving', async () => {
    const page = await openApp(browser, url, { unlocked: true })
    await page.getByText('web-1').dblclick()
    const [id] = await sessionIds(page)
    await fakeEvent(page, id!, { type: 'prompt', prompt_id: 'p2', kind: { type: 'host_key_new', host: 'web-1', key_type: 'ssh-ed25519', fingerprint: 'SHA256:abc' } })
    await page.getByRole('button', { name: 'Connect once' }).click()
    expect(await page.evaluate(() => (window as any).__fake.state.responses)).toMatchObject([{ prompt: 'p2', answer: { accept: true, remember: false } }])
    await fakeEvent(page, id!, { type: 'step', text: 'Signing in as me.' })
    await fakeEvent(page, id!, { type: 'step', text: 'The server accepts: password, keyboard-interactive.' })
    await fakeEvent(page, id!, { type: 'auth_methods', methods: ['password', 'keyboard_interactive'] })
    await fakeEvent(page, id!, { type: 'error', code: 'auth_failed', message: 'web-1 rejected the password.' })
    await seen(page.getByRole('heading', { name: 'Sign-in failed' }))
    await seen(page.getByText('The server accepts: password, keyboard-interactive.'))
    await page.getByRole('button', { name: 'Verification code' }).click()
    expect(await page.evaluate(() => (window as any).__fake.state.calls as string[])).toContain(`reconnect:${id}:keyboard_interactive`)
    await page.close()
  })

  it('S02: right-click on a host card opens the menu; Connect opens a session; the tab menu closes others', async () => {
    const page = await openApp(browser, url, { unlocked: true })
    await page.getByText('web-1').click({ button: 'right' })
    for (const name of ['Connect', 'Connect with SFTP', 'Add to workspace', 'Edit', 'Move to group', 'Copy address', 'Delete'])
      await seen(page.getByRole('menuitem', { name, exact: true }))
    await page.getByRole('menuitem', { name: 'Connect', exact: true }).click()
    await seen(page.getByRole('heading', { name: 'Connecting to web-1…' }))
    expect(await sessionIds(page)).toHaveLength(1)
    // Right-click the tab: Rename, Close, Duplicate tab, Close others.
    await page.getByRole('tab', { name: 'web-1' }).click({ button: 'right' })
    await seen(page.getByRole('menuitem', { name: /^Rename/ }))
    for (const name of ['Close', 'Duplicate tab', 'Close others'])
      await seen(page.getByRole('menuitem', { name, exact: true }))
    await page.close()
  })

  it('F12: a changed key blocks; Close is the default and replacing needs the typed name', async () => {
    const page = await openApp(browser, url, { unlocked: true })
    await page.getByText('web-1').dblclick()
    const [id] = await sessionIds(page)
    await fakeEvent(page, id!, { type: 'prompt', prompt_id: 'p9', kind: { type: 'host_key_changed', host: 'web-1', key_type: 'ssh-ed25519', old_fingerprint: 'SHA256:old', new_fingerprint: 'SHA256:new' } })
    await seen(page.getByRole('heading', { name: 'This server\'s identity changed' }))
    await seen(page.getByText('SHA256:old'))
    await focused(page.getByRole('button', { name: 'Close', exact: true }))
    await page.getByRole('button', { name: 'Replace saved key…' }).click()
    const confirm = page.getByRole('button', { name: /Replace and connect/ })
    await enabled(confirm, false)
    await page.getByRole('textbox').last().fill('web-1')
    await enabled(confirm, true)
    await confirm.click()
    const answered = await page.evaluate(() => (window as any).__fake.state.responses)
    expect(answered).toMatchObject([{ prompt: 'p9', answer: { accept: true, text: 'web-1' } }])
    await page.close()
  })

  it('a live terminal is not unmounted when you switch tabs, and the shortcuts work', async () => {
    const page = await openApp(browser, url, { unlocked: true })
    await page.getByText('web-1').dblclick()
    const [id] = await sessionIds(page)
    await fakeEvent(page, id!, { type: 'state', state: 'ready' })
    await seen(page.locator('.xterm').first())
    await page.evaluate(() => { (document.querySelector('.xterm') as any).__marker = 'same-node' })
    await page.getByRole('tab', { name: 'Vault' }).click()
    await page.getByRole('tab', { name: 'web-1' }).click()
    expect(await page.evaluate(() => (document.querySelector('.xterm') as any).__marker)).toBe('same-node')
    // Ctrl+Tab cycles tabs: web-1 is the last one, so the next is the Vault.
    await page.keyboard.press('Control+Tab')
    await hasAttr(page.getByRole('tab', { name: 'Vault' }), 'aria-selected', 'true')
    // The command palette opens with Ctrl/Cmd+K.
    await page.getByRole('tab', { name: 'Vault' }).click() // out of the terminal, where Ctrl belongs to the shell
    await page.keyboard.press('ControlOrMeta+k')
    await seen(page.getByRole('dialog'))
    await page.close()
  })
})
