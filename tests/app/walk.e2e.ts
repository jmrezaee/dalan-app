import { execSync } from 'node:child_process'
import { addHost, button, byLabel, byText, createVault, heading, openHost, pick, servers, type, typeInTerminal, waitForTerminal } from './support'
import { startChapter, step } from './walkStep'

// `pnpm walk:app`: the story from tests/walk on the REAL app and the REAL Docker servers (replica/testing/test-plan.md
// section 8.5). The chapters that need fake-only controls (the page clock, a scripted server) are left out, and
// the ones that need a real server use `docker restart` and `scripts/test-servers.sh rotate`.
// Videos and screenshots go to test-results/app and walk-results; the gallery is the same as `pnpm walk`.
describe('the walkthrough on the real app', function () {
  // Creating the vault (Argon2 in a debug build, inside a container) has taken from a few seconds to several minutes here.
  this.timeout(900_000)

  const a = servers().a!
  const rot = servers().rot!

  it('Chapter 1: first run (F10, S24)', async () => {
    startChapter(1, 'First run')
    await step({ id: '01-01', flow: 'F10', screen: 'S24', caption: 'A new install asks for a master password' }, async () => {
      await $('#mp').waitForDisplayed()
      await $('#mp2').waitForDisplayed()
    })
    await step({ id: '01-02', flow: 'F10', screen: 'S24', caption: 'Create the vault' }, async () => {
      await createVault()
    })
  })

  it('Chapter 5: connect (F01, S18)', async () => {
    startChapter(5, 'Connect')
    await step({ id: '05-01', flow: 'F03', screen: 'S03', caption: 'Add the server: address, user and password' }, async () => {
      await addHost({ address: a.net_host, port: a.net_port, user: a.user!, password: a.password, label: 'server a' })
    })
    await step({ id: '05-02', flow: 'F01', screen: 'S18', caption: 'A new server asks you to trust its key' }, async () => {
      await openHost('server a')
      await heading('First time connecting to server a').waitForDisplayed()
    })
    await step({ id: '05-03', flow: 'F01', screen: 'S18', caption: 'Trust and connect: a real shell answers' }, async () => {
      await button('Trust and connect').click()
      await waitForTerminal('$')
      await typeInTerminal('echo walk-$((6*7))')
      await waitForTerminal('walk-42')
    })
  })

  it('Chapter 6: sign-in problems (F01, S18)', async () => {
    startChapter(6, 'Sign-in problems')
    await step({ id: '06-01', flow: 'F01', screen: 'S18', caption: 'A wrong saved password fails with a way to try again' }, async () => {
      await $('//*[@role="tab"][contains(normalize-space(.),"Vault")]').click()
      await addHost({ address: a.net_host, port: a.net_port, user: a.user!, password: 'not-the-password', label: 'wrong pw' })
      await openHost('wrong pw')
      await heading('Sign-in failed').waitForDisplayed({ timeout: 30_000 })
      await button('Try again').waitForDisplayed()
    })
  })

  it('Chapters 10 and 11: a restart and a rekey (F11, F12)', async () => {
    startChapter(10, 'Drops')
    await step({ id: '10-01', flow: 'F11', screen: 'S18', caption: 'A server of its own for the drop and the rekey' }, async () => {
      await $('//*[@role="tab"][contains(normalize-space(.),"Vault")]').click()
      await addHost({ address: rot.net_host, port: rot.net_port, user: rot.user!, password: rot.password, label: 'rot' })
      await openHost('rot')
      await heading('First time connecting to rot').waitForDisplayed()
      await button('Trust and connect').click()
      await waitForTerminal('$')
    })
    await step({ id: '10-02', flow: 'F11', screen: 'S18', caption: 'The server restarts: the session says it closed' }, async () => {
      execSync('docker restart -t 1 dalan-test-rot', { stdio: 'ignore' })
      await heading('Connection closed').waitForDisplayed({ timeout: 60_000 })
    })
    await step({ id: '10-03', flow: 'F11', screen: 'S18', caption: 'Reconnect brings the same tab back' }, async () => {
      await browser.pause(8000)
      await button('Reconnect').click()
      await waitForTerminal('$', 40_000)
      await byLabel('rot: connected').waitForDisplayed()
    })

    startChapter(11, 'Identity changed')
    await step({ id: '11-01', flow: 'F12', screen: 'S18', caption: 'The server gets a new host key: connecting is blocked' }, async () => {
      await $('//button[@aria-label="Close rot"]').click()
    await button('Close tab').click() // a live shell asks before it closes
      execSync('scripts/test-servers.sh rotate rot', { cwd: '/work', stdio: 'ignore' })
      await $('//*[@role="tab"][contains(normalize-space(.),"Vault")]').click()
      await openHost('rot')
      await heading("This server's identity changed").waitForDisplayed({ timeout: 30_000 })
      expect(await (await button('Close')).isFocused()).toBe(true)
    })
  })

  it('Chapter 12: tunnels (F06, S09, S10)', async () => {
    startChapter(12, 'Tunnels')
    await step({ id: '12-01', flow: 'F06', screen: 'S10', caption: 'A local tunnel to the web server, through server a' }, async () => {
      await $('//*[@role="tab"][contains(normalize-space(.),"Vault")]').click()
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
    })
    await step({ id: '12-02', flow: 'F06', screen: 'S09', caption: 'Start it, and a real request comes back from the server' }, async () => {
      await button('Start tunnel').click()
      await $('//*[starts-with(@aria-label,"web through a: Active")]').waitForDisplayed({ timeout: 40_000 })
      expect(execSync('curl -s http://127.0.0.1:15080/').toString()).toContain('Welcome to nginx')
    })
  })
})
