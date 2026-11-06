import { execSync } from 'node:child_process'
import { addHost, button, byText, createVault, heading, pick, servers, type } from '../support'

// F06: a local tunnel through server a to the nginx container, used with curl from outside the app.
describe('F06: a local tunnel to a web server', () => {
  it('starts, carries a real request, and stops', async () => {
    const a = servers().a!
    await createVault()
    await addHost({ address: a.net_host, port: a.net_port, user: a.user!, password: a.password, label: 'server a' })

    await $('nav[aria-label="Vault"]').$('//*[normalize-space(.)="Port forwarding"]').click()
    await $('//button[contains(normalize-space(.),"New rule") or contains(normalize-space(.),"New tunnel") or contains(normalize-space(.),"Add rule")]').click()
    await $('//*[@role="radio"][contains(normalize-space(.),"Local")]').click()
    await button('Next').click()
    await type($('#fw-label'), 'web through a')
    await pick($('#fw-host'), 'server a')
    await type($('#fw-bport'), '15080')
    await type($('#fw-dest'), 'web')
    await type($('#fw-dport'), '80')
    await button('Next').click()
    await button('Save').click()
    await byText('web through a').waitForDisplayed()

    await button('Start tunnel').click()
    // The first connection through a new tunnel asks about the server's key.
    await heading('Starting web through a').waitForDisplayed({ timeout: 20_000 })
    await $('//button[normalize-space(.)="Trust and connect"]').click()
    await $('//*[@aria-label and starts-with(@aria-label,"web through a: Active")]').waitForDisplayed({ timeout: 30_000 })

    const page = execSync('curl -s http://127.0.0.1:15080/').toString()
    expect(page).toContain('Welcome to nginx')

    await button('Stop tunnel').click()
    await $('//*[@aria-label="web through a: Stopped"]').waitForDisplayed()
  })
})
