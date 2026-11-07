import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, fill, dblclick, press, step } from './step'
import { unlock } from '../e2e/harness'
import { terminalText } from './helpers'

// Chapter 5 (F01, S18, S20): connect to a new server, trust its key, a scripted login, the find bar, and the side panel.
chapter(5, { scenario: 'team' }, async (page, fake) => {
  await unlock(page, MASTER)
  let id = ''
  let connected: Promise<string>

  await step(page, { id: '05-01', flow: 'F01', screen: 'S18', caption: 'Double-click a server: Dalan starts connecting' }, async () => {
    await dblclick(page, page.getByText('roaming laptop', { exact: true }).first())
    await expect(page.getByRole('heading', { name: 'Connecting to roaming laptop…' })).toBeVisible()
    id = (await fake.sessionIds())[0]!
    await fake.session(id).event({ type: 'step', text: 'Connecting to laptop.example.com:22.' })
    await fake.session(id).event({ type: 'step', text: 'Connected. Negotiating encryption.' })
    await click(page, page.getByRole('button', { name: 'Show details' }))
    await expect(page.getByText('Connected. Negotiating encryption.')).toBeVisible()
  })

  await step(page, { id: '05-02', flow: 'F01', screen: 'S18', caption: 'A new server asks you to trust its key' }, async () => {
    connected = fake.session(id).connect()
    await expect(page.getByRole('heading', { name: 'First time connecting to roaming laptop' })).toBeVisible()
    await expect(page.getByText('SHA256:fake-roaming laptop')).toBeVisible()
  })

  await step(page, { id: '05-03', flow: 'F01', screen: 'S18', caption: 'Trust and connect: a live shell prints its login' }, async () => {
    await click(page, page.getByRole('button', { name: 'Trust and connect' }))
    expect(await connected!).toBe('ready')
    await fake.session(id).runScript('ubuntu-login')
    await expect.poll(() => terminalText(page)).toContain('nothing to commit, working tree clean')
  })

  await step(page, { id: '05-04', flow: 'F01', screen: 'S18', caption: 'Find in the terminal' }, async () => {
    await click(page, page.locator('.xterm').first())
    await press(page, process.platform === 'darwin' ? 'Meta+f' : 'Control+Shift+f') // off a Mac, plain Ctrl+F belongs to the shell
    await fill(page, page.getByLabel('Find in terminal'), 'git')
    await press(page, 'Enter')
    await expect(page.getByLabel('Find in terminal')).toBeVisible()
    await press(page, 'Escape')
  })

  await step(page, { id: '05-05', flow: 'F01', screen: 'S20', caption: 'The side panel: snippets, history, theme' }, async () => {
    await click(page, page.getByRole('button', { name: /^Snippets/ }))
    await expect(page.getByRole('tab', { name: 'History', exact: true })).toBeVisible()
    await click(page, page.getByRole('tab', { name: 'Theme', exact: true }))
    await click(page, page.getByRole('button', { name: /Midnight/ }))
    await expect(page.getByRole('button', { name: /Midnight/ })).toHaveAttribute('aria-pressed', 'true')
  })
})
