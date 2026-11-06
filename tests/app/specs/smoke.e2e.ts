import { addHost, button, byText, createVault, heading, servers, terminalText, typeInTerminal, waitForTerminal } from '../support'

// The first real-app test (F10, F01): a fresh vault, a host on the Docker server a, the host-key question, a real
// shell, and a command whose output comes from the server.
describe('smoke: the real app against server a', () => {
  it('creates a vault, adds a host, trusts its key and runs a command', async () => {
    const a = servers().a!
    await createVault()
    await addHost({ address: a.net_host, port: a.net_port, user: a.user!, password: a.password, label: 'server a' })

    await (await byText('server a')).doubleClick()
    await heading('First time connecting to server a').waitForDisplayed()
    await button('Trust and connect').click()

    await waitForTerminal('$')
    await typeInTerminal('echo real-$((6*7))')
    await waitForTerminal('real-42')
    expect(await terminalText()).toContain('real-42')
  })
})
