import { expect, test } from '../support/fixtures'
import { MASTER } from '../fake/commands/vault'
import { installFakeBackend } from '../fake'

// Accessibility (replica/testing/test-plan.md section 9.3): F01 (connect to a new server) and F03 (the palette) done
// with the keyboard only, with the focus always visible and overlays that take focus themselves.

/** Presses Tab until `match` is focused, so the test finds a control the way a keyboard user does. */
async function tabTo(page: import('@playwright/test').Page, match: (el: Element) => boolean, max = 80) {
  for (let i = 0; i < max; i++) {
    const hit = await page.evaluate(m => (new Function('el', `return (${m})(el)`))(document.activeElement), match.toString())
    if (hit) return
    await page.keyboard.press('Tab')
  }
  throw new Error('Tab never reached the control')
}

/** The focused element shows a ring: an outline or a box shadow that is not "none". */
const focusVisible = async (page: import('@playwright/test').Page) => {
  await page.waitForTimeout(300) // the ring fades in
  return page.evaluate(() => {
  const el = document.activeElement as HTMLElement
  const cs = getComputedStyle(el)
  const outline = cs.outlineStyle !== 'none' && (cs.outlineStyle === 'auto' || parseFloat(cs.outlineWidth) > 0)
  return outline || (cs.boxShadow !== 'none' && cs.boxShadow !== '')
  })
}

test('F01 with the keyboard only: unlock, find a server, connect, trust the key, and the focus is always visible', async ({ page }) => {
  const fake = await installFakeBackend(page, { scenario: 'team', unlocked: false })
  await page.goto('/')

  // Unlock: the password field has focus; type and press Enter.
  await expect(page.getByLabel('Master password')).toBeFocused()
  await page.keyboard.type(MASTER)
  await page.keyboard.press('Enter')
  await expect(page.getByLabel('Vault', { exact: true })).toBeVisible()

  // Tab to the "roaming laptop" card and open it with Enter.
  await tabTo(page, el => el.getAttribute('role') === 'row' && /roaming laptop/.test(el.textContent ?? ''))
  expect(await focusVisible(page)).toBe(true)
  await page.keyboard.press('Enter')
  await expect(page.getByRole('heading', { name: 'Connecting to roaming laptop…' })).toBeVisible()

  // The server's key is new: the question appears and focus moves into it (the terminal behind it is inert).
  const [id] = await fake.sessionIds()
  const connected = fake.session(id!).connect()
  await expect(page.getByRole('heading', { name: 'First time connecting to roaming laptop' })).toBeVisible()
  // Focus lands on Cancel, so a stray Enter never trusts a key.
  await expect(page.getByRole('button', { name: 'Cancel' })).toBeFocused()
  await tabTo(page, el => /Trust and connect/.test(el.textContent ?? ''), 8)
  expect(await focusVisible(page)).toBe(true)
  await page.keyboard.press('Enter')
  expect(await connected).toBe('ready')
})

test('F03 with the keyboard only: the palette opens, filters, and connects', async ({ page }) => {
  const fake = await installFakeBackend(page, { scenario: 'team', unlocked: true })
  await page.goto('/')
  await expect(page.getByLabel('Vault', { exact: true })).toBeVisible()
  await page.keyboard.press('ControlOrMeta+k')
  await expect(page.getByPlaceholder('Search hosts, tabs and actions')).toBeFocused()
  await page.keyboard.type('bastion')
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('Enter')
  await expect(page.getByRole('heading', { name: 'Connecting to bastion…' })).toBeVisible()
  expect(await fake.sessionIds()).toHaveLength(1)
  // Escape closes the palette without doing anything.
  await page.keyboard.press('ControlOrMeta+k')
  await page.keyboard.press('Escape')
  await expect(page.getByPlaceholder('Search hosts, tabs and actions')).toHaveCount(0)
  expect(await fake.sessionIds()).toHaveLength(1)
})

test('overlays: the safe button has focus, and Escape on the changed-key block picks the safe action', async ({ page }) => {
  const fake = await installFakeBackend(page, { scenario: 'team', unlocked: true })
  await page.goto('/')
  await page.getByRole('row').filter({ has: page.getByText('bastion', { exact: true }) }).first().dblclick()
  const [id] = await fake.sessionIds()
  await fake.session(id!).ask({ type: 'host_key_changed', host: 'bastion', key_type: 'ssh-ed25519', old_fingerprint: 'SHA256:old', new_fingerprint: 'SHA256:new' }, 'chg')
  await expect(page.getByRole('button', { name: 'Close', exact: true })).toBeFocused()
  await page.keyboard.press('Escape')
  await expect.poll(async () => (await fake.responses()).find(r => r.prompt === 'chg')?.answer).toMatchObject({ accept: false })
})

test('every button, tab, row and switch you can Tab to shows a focus ring (Hosts, Keychain, Settings)', async ({ page }) => {
  test.setTimeout(120_000) // 3 screens x 40 Tabs x 220 ms is about 26 s on a fast machine; a shared CI runner needs more than the default 30 s
  await installFakeBackend(page, { scenario: 'team', unlocked: true })
  await page.goto('/')
  await expect(page.getByLabel('Vault', { exact: true })).toBeVisible()
  const missing: string[] = []
  for (const open of [
    async () => {},
    async () => { await page.getByLabel('Vault', { exact: true }).getByText('Keychain', { exact: true }).click() },
    async () => { await page.keyboard.press('ControlOrMeta+,') },
  ]) {
    await open()
    await page.locator('body').click({ position: { x: 5, y: 5 } })
    for (let i = 0; i < 40; i++) {
      await page.keyboard.press('Tab')
      await page.waitForTimeout(220) // the ring fades in
      const r = await page.evaluate(() => {
        const el = document.activeElement as HTMLElement
        if (!el || el === document.body || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName)) return null // text fields show a border instead
        const cs = getComputedStyle(el)
        const ring = (cs.outlineStyle !== 'none' && (cs.outlineStyle === 'auto' || parseFloat(cs.outlineWidth) > 0)) || (cs.boxShadow !== 'none' && cs.boxShadow !== '')
        return ring ? null : `${el.tagName.toLowerCase()}${el.getAttribute('role') ? `[role=${el.getAttribute('role')}]` : ''} "${(el.getAttribute('aria-label') ?? el.textContent ?? '').trim().slice(0, 40)}"`
      })
      if (r) missing.push(r)
    }
  }
  expect([...new Set(missing)]).toEqual([])
})
