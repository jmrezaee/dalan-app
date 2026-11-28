import { execSync } from 'node:child_process'
import { writeFileSync } from 'node:fs'
import { addHost, createVault, jsClick, openHost, servers, type } from '../support'

// P31 on the real app, with the window's native file drop on (dragDropEnabled): the in-app drags still work, because
// they run on pointer events. Each drag is WebDriver pointer actions (press, moves, release), not HTML5 events.
async function drag(from: WebdriverIO.Element, to: WebdriverIO.Element, fx = 0.5) {
  await browser.execute((el: HTMLElement) => el.scrollIntoView({ block: 'center' }), from as unknown as HTMLElement)
  const a = await from.getLocation()
  const as = await from.getSize()
  const b = await to.getLocation()
  const bs = await to.getSize()
  const x0 = Math.round(a.x + as.width / 2)
  const y0 = Math.round(a.y + as.height / 2)
  await browser.action('pointer', { parameters: { pointerType: 'mouse' } })
    .move({ x: x0, y: y0, origin: 'viewport' })
    .down()
    .move({ x: x0 + 8, y: y0 + 2, duration: 60, origin: 'viewport' })
    .move({ x: Math.round(b.x + bs.width * fx), y: Math.round(b.y + bs.height / 2), duration: 250, origin: 'viewport' })
    .up()
    .perform()
  await browser.pause(400)
}
const tabTexts = () => browser.execute(() => [...document.querySelectorAll('[role="tab"]')].map(t => (t as HTMLElement).innerText.trim()))
const tabAt = (label: string) => $(`//*[@role="tab"][contains(normalize-space(.),"${label}")]/..`)

describe('P31: in-app drags in the real window with the native file drop on', () => {
  before(async () => {
    await createVault()
    const a = servers().a!
    const c = servers().c!
    await addHost({ address: a.net_host, port: a.net_port, user: a.user!, password: a.password, label: 'server a' })
    await addHost({ address: c.net_host, port: c.net_port, user: c.user!, password: c.password, label: 'server c' })
  })

  it('a host card dropped on a group moves into it', async () => {
    await $('//button[@aria-label="More create options"]').click()
    await $('//*[@role="menuitem"][normalize-space(.)="New group"]').click()
    await type($('#group-label'), 'Lab')
    await $('//*[@role="dialog"]//button[normalize-space(.)="Save"]').click()
    await $('#group-label').waitForDisplayed({ reverse: true, timeout: 10_000 }).catch(() => {})
    const group = await $('//*[normalize-space(text())="Lab"]/ancestor::div[contains(@class,"rounded-lg")][1]')
    await group.waitForDisplayed()
    await browser.pause(600)
    await drag(await $('//*[@role="row"][.//*[normalize-space(text())="server c"]]'), group)
    await browser.waitUntil(async () => (await group.getText()).includes('1 host'), { timeout: 10_000, timeoutMsg: 'server c did not move into Lab' })
  })

  it('a tab dropped on the left quarter of another goes before it', async () => {
    await openHost('server a')
    for (const t of ['Trust and connect', 'Connect']) {
      const b = await $(`//button[normalize-space(.)="${t}"]`)
      if (await b.waitForDisplayed({ timeout: 8_000 }).then(() => true, () => false)) await b.click()
    }
    await $('//*[@role="tab"][normalize-space(.)="Vault"]').click()
    await $('//*[normalize-space(text())="Lab"]').click()
    await openHost('server c')
    for (const t of ['Trust and connect', 'Connect']) {
      const b = await $(`//button[normalize-space(.)="${t}"]`)
      if (await b.waitForDisplayed({ timeout: 8_000 }).then(() => true, () => false)) await b.click()
    }
    await browser.waitUntil(async () => (await tabTexts()).some(t => t.includes('server c')), { timeout: 20_000 })
    const before = (await tabTexts()).filter(t => /server [ac]/.test(t)).map(t => t.match(/server [ac]/)![0])
    expect(before).toEqual(['server a', 'server c'])
    await drag(await tabAt('server c'), await tabAt('server a'), 0.1)
    const after = (await tabTexts()).filter(t => /server [ac]/.test(t)).map(t => t.match(/server [ac]/)![0])
    expect(after).toEqual(['server c', 'server a'])
  })

  it('a file dragged from this computer onto server a is copied there', async () => {
    writeFileSync('/root/p31-drag.txt', 'dragged across\n')
    await $('//*[@role="tab"][contains(normalize-space(.),"SFTP")]').click()
    await $('(//button[contains(normalize-space(.),"Choose a place")])[1]').click()
    await $('//*[@role="menuitem"][normalize-space(.)="server a"]').click()
    const trust = await $('//button[normalize-space(.)="Trust and connect"]')
    if (await trust.waitForDisplayed({ timeout: 10_000 }).then(() => true, () => false)) await trust.click()
    const right = await $('[data-testid="sftp-pane-right"]')
    await right.$('.//*[normalize-space(text())="sshd.pid"]').waitForDisplayed({ timeout: 40_000 })
    const left = await $('[data-testid="sftp-pane-left"]')
    await jsClick(left.$('.//button[@aria-label="Refresh"]'))
    const row = await left.$('.//*[@role="row"][.//*[normalize-space(text())="p31-drag.txt"]]')
    await row.waitForDisplayed({ timeout: 15_000 })
    // WebKitGTK's driver can't act on a <tr> itself: press on its first cell, release over the other pane.
    await drag(await row.$('td:first-child'), right)
    await browser.waitUntil(() => {
      try { return execSync('docker exec dalan-test-ssh cat /config/p31-drag.txt', { stdio: ['ignore', 'pipe', 'ignore'] }).toString() === 'dragged across\n' }
      catch { return false }
    }, { timeout: 30_000, timeoutMsg: 'the dragged file never arrived on server a' })
  })
})
