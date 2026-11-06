import { execSync } from 'node:child_process'
import { addHost, button, createVault, heading, servers, waitForTerminal, openHost } from '../support'

// F12: after a server's host key changes, Dalan blocks the connection (a real sshd, a real rekey).
describe('F12: a rotated host key', () => {
  it('trusts the first key, then blocks the changed one with Close as the default', async () => {
    const r = servers().rot!
    await createVault()
    await addHost({ address: r.net_host, port: r.net_port, user: r.user!, password: r.password, label: 'rot' })
    await openHost('rot')
    await heading('First time connecting to rot').waitForDisplayed()
    await button('Trust and connect').click()
    await waitForTerminal('$')
    await $('//button[@aria-label="Close rot"]').click()
    await button('Close tab').click() // a live shell asks before it closes

    execSync('scripts/test-servers.sh rotate rot', { cwd: '/work', stdio: 'ignore' })

    await openHost('rot')
    await heading("This server's identity changed").waitForDisplayed({ timeout: 30_000 })
    expect(await (await button('Close')).isFocused()).toBe(true)
  })
})
