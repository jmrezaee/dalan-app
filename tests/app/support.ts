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
export async function addHost(h: { address: string, port?: number, user: string, password?: string, label?: string }) {
  await (await $('//button[contains(normalize-space(.),"Add host") or contains(normalize-space(.),"New host")]')).click()
  await type($('#host-address'), h.address)
  if (h.port) await type($('#host-port'), String(h.port))
  if (h.label) await type($('#host-label'), h.label)
  await type($('#host-user'), h.user)
  if (h.password) await type($('#host-password'), h.password)
  await button('Save').click()
  await byText(h.label ?? h.address).waitForDisplayed()
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
