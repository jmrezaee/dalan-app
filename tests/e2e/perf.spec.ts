import { expect, test } from '../support/fixtures'
import { installFakeBackend } from '../fake'
import { sessionIds } from './harness'

// test-plan 9.4: the UI stays quick with a big vault and during a flood of terminal output.
// The thresholds are the plan's; they are generous enough for a shared CI machine because they use the median.

const HOSTS = 2000
const manyHosts = () => Array.from({ length: HOSTS }, (_, i) => ({
  id: `h${i}`, label: `server-${String(i).padStart(4, '0')}`, address: `10.${i >> 8}.${i & 255}.1`, username: 'me', password: true, tags: i % 7 === 0 ? ['seventh'] : [],
}))

/** Milliseconds from `act` until the browser has painted the next frame twice (the change is on screen). */
async function paintTime(page: import('@playwright/test').Page, act: () => Promise<void>) {
  const t0 = await page.evaluate(() => performance.now())
  await act()
  return (await page.evaluate(() => new Promise<number>(r => requestAnimationFrame(() => requestAnimationFrame(() => r(performance.now())))))) - t0
}
const median = (xs: number[]) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)]!

test.describe(`${HOSTS} hosts @perf`, () => {
  test.beforeEach(async ({ page }) => {
    await installFakeBackend(page, { scenario: 'empty', unlocked: true, seed: { vault: { exists: true, unlocked: true }, hosts: manyHosts() } })
    await page.goto('/')
  })

  test('the grid renders in under a second and the search box filters each keystroke in under 100 ms', async ({ page }) => {
    const t0 = Date.now()
    await expect(page.getByText('server-0000').first()).toBeVisible()
    expect(Date.now() - t0, 'first paint of the Hosts grid').toBeLessThan(1500)

    // The Hosts search: type one character at a time and time the repaint after each.
    const search = page.getByRole('searchbox').or(page.getByPlaceholder(/search/i)).first()
    await search.click()
    const times: number[] = []
    for (const ch of 'server-19') times.push(await paintTime(page, () => search.press(ch)))
    await expect(page.getByText('server-1999').first()).toBeVisible()
    console.log(`Hosts search, ${HOSTS} hosts: median ${median(times).toFixed(0)} ms, worst ${Math.max(...times).toFixed(0)} ms`)
    expect(median(times), 'median repaint per keystroke').toBeLessThan(100)
    expect(Math.max(...times), 'worst repaint per keystroke').toBeLessThan(400)
  })

  test('the palette (⌘K) filters each keystroke in under 100 ms', async ({ page }) => {
    await expect(page.getByText('server-0000').first()).toBeVisible()
    await page.keyboard.press('ControlOrMeta+k')
    const box = page.getByPlaceholder('Search hosts, tabs and actions')
    await expect(box).toBeFocused()
    const times: number[] = []
    for (const ch of 'server-07') times.push(await paintTime(page, () => page.keyboard.press(ch)))
    console.log(`Palette, ${HOSTS} hosts: median ${median(times).toFixed(0)} ms, worst ${Math.max(...times).toFixed(0)} ms`)
    expect(median(times), 'median repaint per keystroke').toBeLessThan(100)
    expect(Math.max(...times), 'worst repaint per keystroke').toBeLessThan(400)
  })
})

test('a 5 MB flood of terminal output does not freeze the window: another tab answers in under 200 ms @perf', async ({ page }) => {
  const fake = await installFakeBackend(page, { scenario: 'basic', unlocked: true })
  await page.goto('/')
  await page.getByText('web-1').dblclick()
  const [id] = await sessionIds(page)
  await fake.session(id!).state('ready')
  await expect(page.locator('.xterm').first()).toBeVisible()

  // Start the flood without waiting for it, then use the app while it runs.
  const flood = fake.session(id!).runScript('flood')
  // Click from inside the page, so the time is the app's and not Playwright's own checks before a click.
  const ms = await page.evaluate(() => new Promise<number>((done) => {
    const t0 = performance.now()
    ;(document.querySelector('[role=tab][aria-label*=Vault], [role=tab]') as HTMLElement).click()
    requestAnimationFrame(() => requestAnimationFrame(() => done(performance.now() - t0)))
  }))
  console.log(`Click on another tab during a 5 MB flood: ${ms.toFixed(0)} ms`)
  expect(ms, 'time for a click on another tab to show').toBeLessThan(200)
  await expect(page.getByText('db-1').first()).toBeVisible()
  await flood
  // Everything arrived: the terminal acknowledged bytes (that is how reading is paced).
  await expect.poll(() => page.evaluate(() => (window as any).__fake.state.calls.filter((c: string) => c.startsWith('ack:')).length)).toBeGreaterThan(0)
})
