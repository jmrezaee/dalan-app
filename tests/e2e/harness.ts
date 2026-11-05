import { createServer, type Server } from 'node:http'
import { readFile } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { extname, join, normalize } from 'node:path'
import { expect } from 'vitest'
import { chromium, type Browser, type Locator, type Page } from 'playwright'
import { installFakeBackend, type FakeHost } from './fakeBackend'

const ROOT = join(process.cwd(), '.output', 'public')
const TYPES: Record<string, string> = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.woff2': 'font/woff2', '.svg': 'image/svg+xml', '.png': 'image/png' }

/** Serves the built SPA. Unknown paths get index.html, like Tauri does. */
export async function serveApp(): Promise<{ server: Server, url: string }> {
  if (!existsSync(join(ROOT, 'index.html'))) throw new Error('Run `pnpm build` first: .output/public/index.html is missing.')
  const server = createServer(async (req, res) => {
    const clean = normalize(decodeURIComponent((req.url ?? '/').split('?')[0]!)).replace(/^(\.\.[/\\])+/, '')
    let file = join(ROOT, clean)
    if (!existsSync(file) || clean.endsWith('/')) file = join(ROOT, 'index.html')
    try {
      res.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream' })
      res.end(await readFile(file))
    }
    catch {
      res.writeHead(404).end()
    }
  })
  await new Promise<void>(r => server.listen(0, '127.0.0.1', r))
  const addr = server.address()
  return { server, url: `http://127.0.0.1:${typeof addr === 'object' && addr ? addr.port : 0}` }
}

/** Locally this drives the installed Google Chrome; in CI set PW_CHANNEL= (empty) to use Playwright's own Chromium. */
export function launchBrowser(): Promise<Browser> {
  const channel = process.env.PW_CHANNEL ?? 'chrome'
  return chromium.launch(channel ? { channel } : {})
}

export const HOSTS: FakeHost[] = [
  { id: 'h1', label: 'web-1', address: '10.0.0.1' },
  { id: 'h2', label: 'db-1', address: '10.0.0.2', port: 2222 },
]

/** A page with the fake backend installed and the app open on the unlock screen (or the vault, if `unlocked`). */
export async function openApp(browser: Browser, url: string, opts: { unlocked?: boolean, hosts?: FakeHost[] } = {}): Promise<Page> {
  const page = await browser.newPage({ viewport: { width: 1200, height: 780 } })
  await installFakeBackend(page, opts.hosts ?? HOSTS, opts.unlocked ?? false)
  await page.goto(url)
  return page
}

export async function unlock(page: Page, password = 'correct-horse') {
  await page.getByLabel('Master password').fill(password)
  await page.getByRole('button', { name: 'Unlock' }).click()
}

/** The fake session ids in the order they were opened. */
export const sessionIds = (page: Page) => page.evaluate(() => (window as any).__fake.sessionIds() as string[])
export const fakeEvent = (page: Page, id: string, e: unknown) => page.evaluate(([i, ev]) => (window as any).__fake.event(i, ev), [id, e] as const)
export const fakeOutput = (page: Page, id: string, text: string) => page.evaluate(([i, t]) => (window as any).__fake.output(i, t), [id, text] as const)

// Small assertions on locators (Playwright's own `expect` isn't installed; these retry until the timeout, too).
export const seen = (l: Locator) => l.first().waitFor({ state: 'visible', timeout: 10_000 })
export const gone = (l: Locator) => expect.poll(() => l.count(), { timeout: 10_000 }).toBe(0)
export const focused = (l: Locator) => expect.poll(() => l.first().evaluate(el => el === document.activeElement), { timeout: 10_000 }).toBe(true)
export const enabled = (l: Locator, on: boolean) => expect.poll(() => l.first().isEnabled(), { timeout: 10_000 }).toBe(on)
export const hasText = (l: Locator, text: string) => expect.poll(() => l.first().innerText(), { timeout: 10_000 }).toContain(text)
export const hasAttr = (l: Locator, name: string, value: string) => expect.poll(() => l.first().getAttribute(name), { timeout: 10_000 }).toBe(value)
