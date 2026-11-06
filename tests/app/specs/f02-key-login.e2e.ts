import { addHost, button, createVault, openHost, pick, servers, terminalText, type, typeInTerminal, waitForTerminal } from '../support'

// F02: generate a key, install it on a real server, then sign in with that key and no password.
describe('F02: a generated key', () => {
  it('is installed on server a and signs in on its own', async () => {
    const a = servers().a!
    await createVault()
    await addHost({ address: a.net_host, port: a.net_port, user: a.user!, password: a.password, label: 'server a' })

    await $('nav[aria-label="Vault"]').$('//*[normalize-space(.)="Keychain"]').click()
    await button('Generate key').click()
    await type($('#gen-label'), 'f02 key')
    await button('Generate and save').click()
    await $('//*[normalize-space(text())="f02 key"]').waitForDisplayed()
    await button('Generate and save').waitForDisplayed({ reverse: true }) // the dialog has closed
    await browser.pause(500)

    await $('//button[@aria-label="Key actions"]').click()
    await $('//*[@role="menuitem"][contains(normalize-space(.),"Install on a server")]').click()
    await pick($('#exp-host'), 'server a')
    await button('Install key').click()
    // The first connection to server a asks about its key; a saved password signs in.
    const trust = await $('//button[normalize-space(.)="Trust and connect"]')
    if (await trust.waitForDisplayed({ timeout: 15_000 }).then(() => true, () => false)) await trust.click()
    await button('Done').waitForDisplayed({ timeout: 60_000 })
    await button('Done').click()

    await $('nav[aria-label="Vault"]').$('//*[normalize-space(.)="Hosts"]').click()
    await addHost({ address: a.net_host, port: a.net_port, user: a.user!, label: 'a by key', keyLabel: 'f02 key' })
    await openHost('a by key')
    await waitForTerminal('$', 40_000)
    await typeInTerminal('echo key-$((6*7))')
    await waitForTerminal('key-42')
    expect(await terminalText()).toContain('key-42')
  })
})
