import { execSync } from 'node:child_process'
import { addHost, button, byLabel, createVault, heading, servers, waitForTerminal, openHost } from '../support'

// F11: the connection drops when the server restarts; Reconnect brings the same tab back.
describe('F11: a server restart', () => {
  it('shows the dropped session and reconnects it', async () => {
    const r = servers().rot!
    await createVault()
    await addHost({ address: r.net_host, port: r.net_port, user: r.user!, password: r.password, label: 'rot' })
    await openHost('rot')
    await heading('First time connecting to rot').waitForDisplayed()
    await button('Trust and connect').click()
    await waitForTerminal('$')

    execSync('docker restart -t 1 dalan-test-rot', { stdio: 'ignore' })
    await heading('Connection closed').waitForDisplayed({ timeout: 60_000 })

    // The server is back after a few seconds; Reconnect resumes in the same tab.
    await browser.pause(8000)
    await button('Reconnect').click()
    await waitForTerminal('$', 40_000)
    await byLabel('rot: connected').waitForDisplayed()
  })
})
