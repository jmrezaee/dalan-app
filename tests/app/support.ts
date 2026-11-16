// Helpers for the real-app tests. They talk to the webview through WebDriver, so they use ids, labels and text, and read
// terminals through the test hook (`window.__dalanTerm`) that only the test build has.
import { readFileSync } from 'node:fs'

export interface ServerInfo { name: string, host: string, port: number | null, user?: string, password?: string, net_host: string, net_port: number, fingerprint?: string, key?: string, passphrase?: string, code_command?: string }

/** The Docker servers as the runner sees them: by their names on the Docker network. */
export function servers(): Record<string, ServerInfo> {
  return JSON.parse(readFileSync('/work/.test-servers/servers.json', 'utf8'))
}

export const MASTER = 'correct-horse-staple'

/** XPath for an element with exactly this visible text (buttons, menu items, headings). */
const text = (t: string) => `(//*[normalize-space(text())="${t}"][not(self::script)])[1]`

export const byText = (t: string) => $(text(t))
export const button = (name: string) => $(`//button[normalize-space(.)="${name}" or @aria-label="${name}"]`)
export const heading = (t: string) => $(`//h1[normalize-space(.)="${t}"] | //h2[normalize-space(.)="${t}"]`)
export const byLabel = (label: string) => $(`//*[@aria-label="${label}"]`)

/** Types into a field and waits until the app shows the text (v-model has caught up). */
export async function type(el: ChainablePromiseElement, value: string) {
  await el.waitForDisplayed()
  await el.click()
  await el.clearValue()
  await el.setValue(value)
}

/** Everything the live terminals show, from the xterm buffer. */
export const terminalText = () => browser.execute(() => {
  const terms = Object.values((window as unknown as { __dalanTerm?: Record<string, any> }).__dalanTerm ?? {})
  return terms.map((t) => {
    const b = t.buffer.active
    return Array.from({ length: b.length }, (_, i) => b.getLine(i)?.translateToString(true)).join('\n')
  }).join('\n')
})

/** Waits until a terminal shows `needle`. */
export async function waitForTerminal(needle: string, timeout = 20_000) {
  try {
    await browser.waitUntil(async () => (await terminalText()).includes(needle), { timeout })
  }
  catch {
    throw new Error(`the terminal never showed "${needle}". It shows: ${JSON.stringify((await terminalText()).trim().slice(-300))}`)
  }
}

/** Types into the terminal as a user would (the xterm helper textarea has the focus). */
export async function typeInTerminal(line: string) {
  // xterm reads typing from a hidden textarea; send the keys to that element so they land there.
  const input = await $('.xterm-helper-textarea')
  await input.addValue(line)
  await browser.keys('Enter')
}

/** Creates the vault on a fresh install and lands on the empty Hosts page. */
export async function createVault() {
  await type($('#mp'), MASTER)
  await type($('#mp2'), MASTER)
  await button('Create vault').click()
  await $('nav[aria-label="Vault"]').waitForDisplayed()
}

/** Adds a host with the editor (address, user, password), saved but not connected. */
export async function addHost(h: { address: string, port?: number, user: string, password?: string, label?: string, jump?: string, keyLabel?: string }) {
  await (await $('//button[contains(normalize-space(.),"Add host") or contains(normalize-space(.),"New host")]')).click()
  await type($('#host-address'), h.address)
  if (h.port) await type($('#host-port'), String(h.port))
  if (h.label) await type($('#host-label'), h.label)
  await type($('#host-user'), h.user)
  if (h.password) await type($('#host-password'), h.password)
  if (h.keyLabel) await pick($('#host-key'), h.keyLabel)
  if (h.jump) {
    // The route is set on its own page: Jump hosts, Custom, add the hop, back.
    await $('//button[starts-with(normalize-space(.),"Jump hosts")]').click()
    await $('//*[(self::button or @role="radio") and normalize-space(.)="Custom"]').click()
    await pick($('//*[@role="combobox"][contains(normalize-space(.),"Add jump host")]'), h.jump)
    await $('//button[@aria-label="Back to the host"]').click()
  }
  await button('Save').click()
  await byText(h.label ?? h.address).waitForDisplayed()
  // The sheet slides away; a click that lands while it is still on top is swallowed.
  await $('#host-address').waitForDisplayed({ reverse: true, timeout: 10_000 }).catch(() => {})
  await browser.pause(500)
}

/** Chooses an entry in one of the app's select boxes (a Reka select: open it, click the option). */
export async function pick(trigger: ChainablePromiseElement, option: string) {
  await trigger.click()
  await $(`//*[@role="option"][contains(normalize-space(.),"${option}")]`).click()
}

/** Opens a host's card: focus it with a click and press Enter (a double-click through WebDriver is not reliable). The list
 *  re-renders after a save, so a card found a moment ago can be gone: try again with a fresh lookup. */
export async function openHost(label: string) {
  let last: unknown
  for (let i = 0; i < 6; i++) {
    try {
      const card = await $(`//*[@role="row"][.//*[normalize-space(text())="${label}"]]`)
      await card.waitForDisplayed()
      await card.click()
      await browser.keys('Enter')
      return
    }
    catch (e) {
      last = e
      await browser.pause(400)
    }
  }
  throw last
}

/** Clicks through the page itself. A WebDriver click on a draggable row can start a drag and never become a click. */
export async function jsClick(el: ChainablePromiseElement | WebdriverIO.Element) {
  const e = await el
  await e.waitForExist()
  await browser.execute((node: HTMLElement) => node.click(), e as unknown as HTMLElement)
}

/** The first element matching `xpath` that is on screen. Tabs stay mounted while hidden (terminals, Redis tabs), so the
 *  first match in the page is often one from another tab. */
export async function shown(xpath: string, timeout = 20_000): Promise<WebdriverIO.Element> {
  let found: WebdriverIO.Element | undefined
  await browser.waitUntil(async () => {
    for (const el of await $$(xpath)) {
      if (await el.isDisplayed().catch(() => false)) {
        found = el as unknown as WebdriverIO.Element
        return true
      }
    }
    return false
  }, { timeout, timeoutMsg: `nothing on screen matches ${xpath}` })
  return found!
}

/** Adds a Redis database with the editor (S27): tested first when asked, then saved. */
export async function addDatabase(d: { name: string, address: string, port?: number, password?: string, throughHost?: string, kind?: 'Cluster' | 'Sentinel', test?: boolean }) {
  await (await shown('//button[contains(normalize-space(.),"New database")]')).click()
  await type($('#rdb-name'), d.name)
  if (d.kind) await $(`//*[@role="radio"][normalize-space(.)="${d.kind}"]`).click()
  if (d.throughHost) {
    await $('//*[@role="radio"][normalize-space(.)="Through a host"]').click()
    await pick($('#rdb-host'), d.throughHost)
  }
  await type($('#rdb-address'), d.address)
  if (d.port) await type($('#rdb-port'), String(d.port))
  if (d.password) await type($('#rdb-pass'), d.password)
  if (d.test) {
    await (await shown('//button[normalize-space(.)="Test connection"]')).click()
    await shown('//*[@role="status"][contains(normalize-space(.),"Connected")]', 40_000)
  }
  await (await shown('//button[normalize-space(.)="Save"]')).click()
  await $('#rdb-name').waitForDisplayed({ reverse: true, timeout: 10_000 })
  await browser.pause(500) // the sheet slides away
}

/** Opens a saved database in a tab and waits for its key tree (the connection is real: SSH hops, then Redis). */
export async function openDatabase(name: string, timeout = 60_000) {
  const row = await shown(`//tr[.//*[normalize-space(text())="${name}"]]`)
  // WebKit's driver refuses to click a table row, so the row is focused the way Tab would, and Enter opens it.
  await browser.execute((node: HTMLElement) => node.focus(), row as unknown as HTMLElement)
  await browser.keys('Enter')
  await shown('//*[@role="tree"][@aria-label="Keys"]', timeout)
}

/** Runs a line in the Console of the Redis tab in front and waits for `expectText` in its output. */
export async function consoleRun(line: string, expectText: string) {
  const input = await shown('//input[@aria-label="Command"]')
  // The Console focuses its input when it opens, and WebKit's driver sometimes calls a click on it "intercepted"
  // (it did on GitHub's slower machine). Focusing it from the page is what a click would have done.
  await browser.execute((node: HTMLElement) => node.focus(), input as unknown as HTMLElement)
  await input.setValue(line)
  await browser.keys('Enter')
  await shown(`//*[@role="log"][contains(normalize-space(.),"${expectText}")]`)
}
