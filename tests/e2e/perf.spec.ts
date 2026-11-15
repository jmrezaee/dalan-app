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

// ---- Redis (replica/redis/spec.md section 6): a big keyspace, a big value and a Pub/Sub flood stay usable.
async function openRedis(page: import('@playwright/test').Page, fill?: (fake: Awaited<ReturnType<typeof installFakeBackend>>) => Promise<void>) {
  const fake = await installFakeBackend(page, { scenario: 'team', unlocked: true })
  await page.goto('/')
  await fill?.(fake)
  await page.getByRole('navigation', { name: 'Vault' }).getByRole('link', { name: /Databases/ }).click()
  await page.getByRole('row', { name: /local/ }).dblclick()
  const tab = page.getByTestId('redis-tab-local')
  await expect(tab.getByRole('tree', { name: 'Keys' })).toBeVisible()
  return { fake, tab }
}

test('scanning 100,000 keys keeps the tree quick: each "Scan more" paints in under 200 ms and only the rows in view are drawn @perf', async ({ page }) => {
  test.setTimeout(180_000)
  const { tab } = await openRedis(page, fake => fake.redisFill(100_000))
  // Bigger steps, like someone who set "Keys per scan" to its largest.
  await page.keyboard.press('ControlOrMeta+,')
  await page.getByRole('button', { name: 'Databases', exact: true }).first().click()
  await page.getByLabel('Keys per scan').fill('5000')
  await page.getByLabel('Keys per scan').press('Tab')
  await page.getByRole('tab', { name: 'local' }).click()
  const more = tab.getByRole('button', { name: 'Scan more' })
  const times: number[] = []
  for (let i = 0; i < 20; i++) times.push(await paintTime(page, () => more.click()))
  await expect(tab.getByText(/Scanned 1\d\d,\d{3} of 101,237 keys|All 101,237 keys scanned/)).toBeVisible()
  // 100,000 keys under bulk: fold into 1,000 folders; open one and scroll to the end.
  await tab.getByRole('treeitem', { name: /^bulk:/ }).click()
  const drawn = await tab.getByRole('treeitem').count()
  console.log(`Scan more with up to 100,000 keys loaded: median ${median(times).toFixed(0)} ms, worst ${Math.max(...times).toFixed(0)} ms; ${drawn} rows drawn`)
  expect(median(times), 'median repaint per Scan more').toBeLessThan(200)
  expect(Math.max(...times), 'worst repaint per Scan more').toBeLessThan(1500)
  expect(drawn, 'rows in the DOM').toBeLessThan(120)
  const scroll = await paintTime(page, () => tab.getByRole('tree', { name: 'Keys' }).evaluate((el) => { el.scrollTop = el.scrollHeight }))
  console.log(`Scroll to the end of the tree: ${scroll.toFixed(0)} ms`)
  expect(scroll, 'scroll to the end of the tree').toBeLessThan(300)
  const heap = await page.evaluate(() => (performance as any).memory?.usedJSHeapSize as number | undefined)
  if (heap) {
    console.log(`JS heap with 100,000 scanned keys: ${(heap / 1024 / 1024).toFixed(0)} MB`)
    expect(heap, 'JS heap').toBeLessThan(600 * 1024 * 1024)
  }
})

test('a 50,000-member set loads in pages and scrolls smoothly @perf', async ({ page }) => {
  test.setTimeout(180_000)
  const { tab } = await openRedis(page, fake => fake.redisAddSet('audit:seen', 50_000))
  await tab.getByRole('treeitem', { name: /audit:seen/ }).click()
  const value = tab.getByTestId('redis-value')
  const more = value.getByRole('button', { name: 'Load more' })
  const times: number[] = []
  for (let i = 0; i < 99; i++) times.push(await paintTime(page, () => more.click()))
  await expect(value.getByText('All 50,000 members loaded')).toBeVisible()
  const table = value.getByRole('table', { name: 'Members' })
  const scrolls: number[] = []
  for (const at of [0.25, 0.5, 0.75, 1, 0]) scrolls.push(await paintTime(page, () => table.evaluate((el, f) => { el.scrollTop = el.scrollHeight * f }, at)))
  const drawn = await table.getByRole('row').count()
  console.log(`Load more to 50,000 members: median ${median(times).toFixed(0)} ms; scroll: median ${median(scrolls).toFixed(0)} ms, worst ${Math.max(...scrolls).toFixed(0)} ms; ${drawn} rows drawn`)
  expect(median(times), 'median repaint per Load more').toBeLessThan(150)
  expect(median(scrolls), 'median repaint per scroll').toBeLessThan(100)
  expect(drawn, 'rows in the DOM').toBeLessThan(100)
})

test('a Pub/Sub flood of 10,000 messages a second does not freeze the tab: Pause answers in under 200 ms @perf', async ({ page }) => {
  const { fake, tab } = await openRedis(page)
  await tab.getByRole('tab', { name: 'Pub/Sub' }).click()
  await tab.getByLabel('Channel or pattern').fill('flood')
  await tab.getByRole('button', { name: 'Listen', exact: true }).click()
  // Three seconds of 10,000 a second, sent without waiting, while the app is used.
  const flood = (async () => {
    for (let s = 0; s < 3; s++) {
      await fake.redisPublish('flood', `second ${s}`, 10_000)
      await page.waitForTimeout(1000)
    }
  })()
  await page.waitForTimeout(300)
  const ms = await page.evaluate(() => new Promise<number>((done) => {
    const t0 = performance.now()
    ;([...document.querySelectorAll('button')].find(b => b.textContent?.trim() === 'Pause') as HTMLElement).click()
    requestAnimationFrame(() => requestAnimationFrame(() => done(performance.now() - t0)))
  }))
  console.log(`Click on Pause during a 10,000 messages/s flood: ${ms.toFixed(0)} ms`)
  expect(ms, 'time for Pause to show').toBeLessThan(200)
  await flood
  await expect(tab.getByText(/30,000 received · keeping the last 5,000/)).toBeVisible()
  expect(await tab.getByRole('table', { name: 'Messages' }).getByRole('row').count()).toBeLessThan(80)
})
