import { addHost, button, createVault, heading, openHost, servers, terminalText, typeInTerminal, waitForTerminal } from '../support'

// F05: a server that is only reachable through another one (c is on the Docker network, reachable from a).
describe('F05: a jump host', () => {
  it('connects to the inner server through the bastion, asking about each key', async () => {
    const a = servers().a!
    const c = servers().c!
    await createVault()
    await addHost({ address: a.net_host, port: a.net_port, user: a.user!, password: a.password, label: 'bastion' })
    await addHost({ address: c.net_host, port: c.net_port, user: c.user!, password: c.password, label: 'inner', jump: 'bastion' })

    await openHost('inner')
    await heading('First time connecting to bastion').waitForDisplayed({ timeout: 30_000 })
    await button('Trust and connect').click()
    await heading('First time connecting to inner').waitForDisplayed({ timeout: 30_000 })
    await button('Trust and connect').click()

    await waitForTerminal('$')
    await typeInTerminal('echo through-$((20+22))')
    await waitForTerminal('through-42')
    expect(await terminalText()).toContain('through-42')
  })
})
