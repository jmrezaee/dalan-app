import { expect, test } from '../support/fixtures'
import { enabled, focused, fakeEvent, gone, hasAttr, seen, fakeOutput, openApp, sessionIds, unlock } from './harness'

test.describe('unlock and the connection overlay (F01, F11, F12)', () => {
  test('a wrong master password is refused, the right one opens the vault', async ({ page }) => {
    await openApp(page)
    await unlock(page, 'not-the-password')
    await seen(page.getByText('That password didn\'t work.').first())
    await unlock(page)
    await seen(page.getByText('web-1'))
  })

  test('F01: connecting, trust the key, then a live shell shows output', async ({ page }) => {
    await openApp(page, { unlocked: true })
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
  })

  test('F11: a dropped connection offers Reconnect, and Enter does it', async ({ page }) => {
    await openApp(page, { unlocked: true })
    await page.getByText('web-1').dblclick()
    const [id] = await sessionIds(page)
    await fakeEvent(page, id!, { type: 'state', state: 'ready' })
    await fakeEvent(page, id!, { type: 'state', state: 'disconnected' })
    await seen(page.getByRole('heading', { name: 'Connection closed' }))
    await focused(page.getByRole('button', { name: 'Reconnect' }))
    await page.keyboard.press('Enter')
    const calls = await page.evaluate(() => (window as any).__fake.state.calls as string[])
    expect(calls).toContain(`reconnect:${id}`)
  })

  test('F01: a failed sign-in shows the step log, offers another way, and Connect once answers without saving', async ({ page }) => {
    await openApp(page, { unlocked: true })
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
  })

  test('S02: right-click on a host card opens the menu; Connect opens a session; the tab menu closes others', async ({ page }) => {
    await openApp(page, { unlocked: true })
    await page.getByText('web-1').click({ button: 'right' })
    for (const name of ['Connect', 'Connect with SFTP', 'Add to workspace', 'Edit', 'Duplicate', 'Move to group', 'Copy address', 'Delete'])
      await seen(page.getByRole('menuitem', { name, exact: true }))
    await page.getByRole('menuitem', { name: 'Connect', exact: true }).click()
    await seen(page.getByRole('heading', { name: 'Connecting to web-1…' }))
    expect(await sessionIds(page)).toHaveLength(1)
    // Right-click the tab: Rename, Close, Duplicate tab, Close others.
    await page.getByRole('tab', { name: 'web-1' }).click({ button: 'right' })
    await seen(page.getByRole('menuitem', { name: /^Rename/ }))
    for (const name of ['Close', 'Duplicate tab', 'Close others'])
      await seen(page.getByRole('menuitem', { name, exact: true }))
  })

  test('S20: the side panel has Theme and Font tabs and opens on the tab you used last', async ({ page }) => {
    await openApp(page, { unlocked: true })
    await page.getByText('web-1').dblclick()
    const [id] = await sessionIds(page)
    await fakeEvent(page, id!, { type: 'state', state: 'ready' })
    await page.getByRole('button', { name: /^Snippets/ }).click()
    for (const name of ['Snippets', 'History', 'Theme', 'Font', 'Ask AI'])
      await seen(page.getByRole('tab', { name, exact: true }))
    await page.getByRole('tab', { name: 'Font', exact: true }).click()
    await seen(page.getByLabel('Font family'))
    await page.getByRole('button', { name: /^Snippets/ }).click() // close
    await page.getByRole('button', { name: /^Snippets/ }).click() // open again
    await seen(page.getByLabel('Font family'))
  })

  test('S03: the host editor asks for the address first, shows a draft card, and opens advanced blocks on their own page', async ({ page }) => {
    await openApp(page, { unlocked: true })
    await page.getByRole('button', { name: 'New host', exact: true }).click()
    const address = page.getByLabel('Address')
    const label = page.getByLabel('Label')
    expect((await address.boundingBox())!.y).toBeLessThan((await label.boundingBox())!.y)
    await address.fill('10.1.2.3')
    await seen(page.getByText('Draft', { exact: true })) // behind the sheet, so not reachable by role
    await seen(page.getByPlaceholder('Same as the address'))
    // More: one line per block with a summary; a block opens on its own page with a back arrow.
    await page.getByRole('button', { name: /^Jump hosts/ }).click()
    await seen(page.getByRole('button', { name: 'Back to the host' }))
    await seen(page.getByRole('list', { name: /you/i }))
    await page.getByRole('button', { name: 'Back to the host' }).click()
    await seen(page.getByRole('button', { name: /^Proxy/ }))
  })

  test('F12: a changed key blocks; Close is the default and replacing needs the typed name', async ({ page }) => {
    await openApp(page, { unlocked: true })
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
  })

  test('a live terminal is not unmounted when you switch tabs, and the shortcuts work', async ({ page }) => {
    await openApp(page, { unlocked: true })
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
  })

  test('the test build exposes the terminal buffer, so tests read text instead of screenshots', async ({ page }) => {
    await openApp(page, { unlocked: true })
    await page.getByText('web-1').dblclick()
    const [id] = await sessionIds(page)
    await fakeEvent(page, id!, { type: 'state', state: 'ready' })
    await fakeOutput(page, id!, 'hello buffer\r\n')
    await expect.poll(() => page.evaluate(() => {
      const term = Object.values((window as any).__dalanTerm ?? {})[0] as any
      return term?.buffer.active.getLine(0)?.translateToString(true)
    })).toContain('hello buffer')
    await seen(page.getByTestId('app-tabbar'))
  })
})
