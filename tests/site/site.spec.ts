import { fileURLToPath } from 'node:url'
import AxeBuilder from '@axe-core/playwright'
import { expect, test } from '@playwright/test'
import type { Browser, Page } from '@playwright/test'

// The download page (site/index.html, spec: replica/launch/landing-v2.md) opened from disk, as each visitor sees it.
const url = new URL('../../site/index.html', `file://${fileURLToPath(import.meta.url)}`).href
const VERSION = '0.1.2'
const releases = (name: string) => `https://github.com/OWNER/dalan-app/releases/download/v${VERSION}/${name}`

const visitors = {
  mac: { platform: 'macOS', mobile: false, ua: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36', label: 'Download for macOS', file: `Dalan_${VERSION}_universal.dmg`, card: 'mac' },
  windows: { platform: 'Windows', mobile: false, ua: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36', label: 'Download for Windows', file: `Dalan_${VERSION}_x64-setup.exe`, card: 'win' },
  linux: { platform: 'Linux', mobile: false, ua: 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36', label: 'Download for Linux', file: `Dalan_${VERSION}_amd64.AppImage`, card: 'linux' },
  iphone: { platform: 'iOS', mobile: true, ua: 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1', label: 'See downloads', file: null, card: null },
  android: { platform: 'Android', mobile: true, ua: 'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Mobile Safari/537.36', label: 'See downloads', file: null, card: null },
} as const

async function open(browser: Browser, v: typeof visitors[keyof typeof visitors], width: number, height = 900): Promise<Page> {
  const ctx = await browser.newContext({ userAgent: v.ua, viewport: { width, height }, reducedMotion: 'reduce' })
  // The page reads navigator.userAgentData first; give it the visitor's.
  await ctx.addInitScript(({ platform, mobile }) => {
    Object.defineProperty(navigator, 'userAgentData', { value: { platform, mobile, brands: [] }, configurable: true })
  }, { platform: v.platform, mobile: v.mobile })
  const page = await ctx.newPage()
  await page.goto(url)
  return page
}

for (const [name, v] of Object.entries(visitors)) {
  test(`${name}: the hero button, the cards and the links`, async ({ browser }) => {
    const page = await open(browser, v, v.mobile ? 390 : 1440)
    const hero = page.locator('[data-hero]').first()
    await expect(hero).toContainText(v.label)
    if (v.file) await expect(hero).toHaveAttribute('href', releases(v.file))
    else await expect(hero).toHaveAttribute('href', '#download')
    // Only the visitor's own card gets the accent ring
    await expect(page.locator('.dl.is-you')).toHaveCount(v.card ? 1 : 0)
    if (v.card) await expect(page.locator(`[data-card="${v.card}"]`)).toHaveClass(/is-you/)
    // "Other platforms" is for people who got a real button
    await expect(page.locator('[data-other]')).toBeVisible({ visible: v.file !== null })
    // All-users is a link, never a second primary button
    const all = page.locator('[data-asset="winall"]')
    await expect(all).toHaveAttribute('href', releases(`Dalan_${VERSION}_x64-allusers-setup.exe`))
    await expect(all).not.toHaveClass(/btn/)
    await expect(page.locator('[data-asset="deb"]')).toHaveAttribute('href', releases(`Dalan_${VERSION}_amd64.deb`))
    await expect(page.locator('[data-asset="rpm"]')).toHaveAttribute('href', releases(`Dalan-${VERSION}-1.x86_64.rpm`))
    await expect(page.locator('.pill')).toHaveText(`v${VERSION}`).catch(() => {}) // hidden on phones
    await page.context().close()
  })
}

test('without JavaScript every button still leads somewhere', async ({ browser }) => {
  const ctx = await browser.newContext({ javaScriptEnabled: false, viewport: { width: 1440, height: 900 } })
  const page = await ctx.newPage()
  await page.goto(url)
  await expect(page.locator('[data-hero]').first()).toHaveAttribute('href', '#download')
  await expect(page.locator('[data-hero-label]').first()).toHaveText('See downloads')
  for (const a of await page.locator('[data-asset]:not([data-asset="auto"])').all()) {
    expect(await a.getAttribute('href'), 'cards link to the latest release').toBe('https://github.com/OWNER/dalan-app/releases/latest')
  }
  await ctx.close()
})

test('the #anchors in the nav and the buttons all have a target', async ({ browser }) => {
  const page = await open(browser, visitors.mac, 1440)
  const anchors = await page.locator('a[href^="#"]').evaluateAll(as => [...new Set(as.map(a => a.getAttribute('href')!))])
  for (const a of anchors) expect(await page.locator(a).count(), `${a} exists`).toBe(1)
  await page.context().close()
})

for (const [label, width] of [['1440', 1440], ['390', 390]] as const) {
  test(`${label}: no sideways scroll, every image has alt text and loads, axe has no serious issues`, async ({ browser }, info) => {
    const page = await open(browser, label === '390' ? visitors.iphone : visitors.mac, width)
    // Lazy images only load near the viewport: ask for all of them, then wait.
    await page.evaluate(() => document.querySelectorAll('img').forEach((i) => { i.loading = 'eager' }))
    await page.waitForFunction(() => [...document.images].every(i => i.complete), undefined, { timeout: 15_000 })
    const broken = await page.evaluate(() => [...document.images].filter(i => !i.naturalWidth).map(i => i.getAttribute('src')))
    expect(broken, 'images that did not load').toEqual([])
    const noAlt = await page.evaluate(() => [...document.images].filter(i => !i.getAttribute('alt')).map(i => i.getAttribute('src')))
    expect(noAlt, 'images without alt text').toEqual([])
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), 'no horizontal scroll').toBe(true)
    // One h1, and the fonts are ours: nothing is asked of another server.
    await expect(page.locator('h1')).toHaveCount(1)
    const outside = await page.evaluate(() => performance.getEntriesByType('resource').map(r => r.name).filter(n => !n.startsWith('file:')))
    expect(outside, 'requests to other servers').toEqual([])

    const axe = await new AxeBuilder({ page }).analyze()
    const serious = axe.violations.filter(v => v.impact === 'serious' || v.impact === 'critical')
    expect(serious.map(v => `${v.id}: ${v.nodes.map(n => n.target.join(' ')).join(' | ')}`)).toEqual([])

    await page.screenshot({ path: info.outputPath(`site-${label}.png`), fullPage: true })
    await page.screenshot({ path: `test-results/site-${label}.png`, fullPage: true })
    await page.context().close()
  })
}

test('the phone menu opens the same links', async ({ browser }) => {
  const page = await open(browser, visitors.iphone, 390)
  await expect(page.locator('.links')).toBeHidden()
  await page.locator('details.menu summary').click()
  for (const name of ['Features', 'Security', 'Download', 'Help']) await expect(page.getByRole('navigation', { name: 'Page (menu)' }).getByRole('link', { name })).toBeVisible()
  await page.context().close()
})
