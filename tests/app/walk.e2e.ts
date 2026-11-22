import { execSync } from 'node:child_process'
import { addDatabase, addHost, button, byLabel, byText, consoleRun, createVault, heading, openDatabase, openHost, pick, servers, shown, type, typeInTerminal, waitForTerminal } from './support'
import { startChapter, step } from './walkStep'

// `pnpm walk:app`: the story from tests/walk on the REAL app and the REAL Docker servers (replica/testing/test-plan.md
// section 8.5). The chapters that need fake-only controls (the page clock, a scripted server) are left out, and
// the ones that need a real server use `docker restart` and `scripts/test-servers.sh rotate`.
// Videos and screenshots go to test-results/app and walk-results; the gallery is the same as `pnpm walk`.
describe('the walkthrough on the real app', function () {
  // The whole walk takes a couple of minutes. A step that hangs fails inside 30 s (see wdio.conf.ts), so this is only
  // the ceiling for a slow machine.
  this.timeout(600_000)

  const a = servers().a!
  const rot = servers().rot!

  it('Chapter 1: first run (F10, S24)', async () => {
    startChapter(1, 'First run')
    await step({ id: '01-01', flow: 'F10', screen: 'S24', caption: 'A new install asks for a master password' }, async () => {
      await $('#mp').waitForDisplayed()
      await $('#mp2').waitForDisplayed()
    })
    // No picture for this step: WebKitGTK in the test container never answers a screenshot of the page that follows
    // the vault being created (probed 2026-11-22: scripts, title and size answer at once; six screenshots in a row
    // timed out). It used to cost eight minutes of retries per run for a picture that was never saved.
    await step({ id: '01-02', flow: 'F10', screen: 'S24', caption: 'Create the vault', picture: false }, async () => {
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

  // The Redis servers from tests/servers/compose.yml: `redis` (one server, password test-redis) and the six-node
  // cluster `rc1` to `rc6`. The runner is on their network, so the direct route works; the route through "server a"
  // (added and trusted in chapter 5) is the one a production database takes.
  it('Chapter 18: databases (F13 to F19, S26 to S33)', async () => {
    startChapter(18, 'Databases')
    const vaultTab = () => $('//*[@role="tab"][contains(normalize-space(.),"Vault")]').click()
    const view = async (name: string) => (await shown(`//*[@role="tab"][normalize-space(.)="${name}"]`)).click()
    const redisCli = (args: string) => execSync(`docker exec dalan-test-redis redis-cli -a test-redis --no-auth-warning ${args}`).toString().trim()
    // Its own name per run, and an expiry, so a run leaves nothing behind.
    const key = `walk:greeting:${Date.now()}`

    await step({ id: '18-01', flow: 'F13', screen: 'S27', caption: 'A Redis database: tested against the real server, then saved' }, async () => {
      await vaultTab()
      await $('nav[aria-label="Vault"]').$('//*[normalize-space(.)="Databases"]').click()
      await addDatabase({ name: 'walk-cache', address: 'redis', password: 'test-redis', test: true })
      await shown('//tr[.//*[normalize-space(text())="walk-cache"]]')
    })

    await step({ id: '18-02', flow: 'F13', screen: 'S28', caption: 'Open it: the tab connects and lists the keys' }, async () => {
      await openDatabase('walk-cache')
      await shown('//button[normalize-space(.)="Confirm writes"]')
    })

    await step({ id: '18-03', flow: 'F15', screen: 'S33', caption: 'A new key is reviewed first: the dialog shows the exact command' }, async () => {
      await (await shown('//button[@aria-label="New key"]')).click()
      await type($('#rk-name'), key)
      await type($('#rk-value'), 'hello from the walk')
      await type($('#rk-expiry'), '15m')
      await (await shown('//*[@role="dialog"]//button[normalize-space(.)="Review change"]')).click()
      await shown(`//*[@role="alertdialog"]//*[contains(normalize-space(.),"SET ${key}")]`)
      // Nothing was sent yet: the server does not have the key.
      expect(redisCli(`EXISTS ${key}`)).toBe('0')
      await (await shown('//*[@role="alertdialog"]//button[normalize-space(.)="Create key"]')).click()
      await shown(`//h2[normalize-space(.)="${key}"]`)
      expect(redisCli(`GET ${key}`)).toBe('hello from the walk')
    })

    await step({ id: '18-04', flow: 'F16', screen: 'S30', caption: 'The console: a read runs at once, KEYS is held back' }, async () => {
      await view('Console')
      await consoleRun(`TTL ${key}`, '(integer) ')
      await consoleRun('KEYS *', 'Not sent. KEYS walks every key at once')
    })

    await step({ id: '18-05', flow: 'F15', screen: 'S33', caption: 'Switch it to Read-only: a write from the console is refused before it is sent' }, async () => {
      await (await shown('//button[normalize-space(.)="Confirm writes"]')).click()
      await (await shown('//*[@role="dialog"]//*[@role="radio"][contains(normalize-space(.),"Read-only")]')).click()
      await (await shown('//*[@role="dialog"]//button[normalize-space(.)="Change mode"]')).click()
      await shown('//*[@role="status"][contains(normalize-space(.),"Read-only. Nothing here can change data.")]')
      await consoleRun(`DEL ${key}`, 'walk-cache is read-only in Dalan')
      expect(redisCli(`EXISTS ${key}`)).toBe('1')
    })

    await step({ id: '18-06', flow: 'F13', screen: 'S33', caption: 'The same Redis through the SSH host: Dalan signs in to the host and reaches Redis from there' }, async () => {
      await vaultTab()
      await addDatabase({ name: 'walk-through-a', address: 'redis', password: 'test-redis', throughHost: 'server a' })
      await openDatabase('walk-through-a')
      await view('Console')
      await consoleRun(`GET ${key}`, 'hello from the walk')
    })

    await step({ id: '18-07', flow: 'F19', screen: 'S31', caption: 'A cluster through the host: a write finds its node, and the Server view lists the nodes' }, async () => {
      await vaultTab()
      await addDatabase({ name: 'walk-cluster', address: 'rc1', port: 7000, password: 'test-redis', throughHost: 'server a', kind: 'Cluster' })
      await openDatabase('walk-cluster')
      await view('Console')
      await consoleRun(`SET ${key} cluster-ok EX 600`, 'This changes data')
      await (await shown('//button[normalize-space(.)="Send"]')).click()
      // Read it back by its length (10 characters): the value itself is already on screen in the SET line above.
      await consoleRun(`STRLEN ${key}`, '(integer) 10')
      await view('Server')
      await shown('//table[@aria-labelledby="nodes-title"]//td[normalize-space(.)="primary"]', 30_000)
      await shown('//table[@aria-labelledby="nodes-title"]//td[contains(normalize-space(.),"5460")]')
    })
  })
})
