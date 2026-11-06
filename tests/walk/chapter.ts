import type { Page } from '@playwright/test'
import { test } from '../support/fixtures'
import { installFakeBackend, type Fake, type FakeOptions } from '../fake'
import { MASTER } from '../fake/commands/vault'
import { CHAPTERS, selected } from './chapters'
import { startChapter } from './step'

export { MASTER }

/** One chapter of the story: a fresh page with the fake backend, the scenario it starts from, and the steps. */
export function chapter(n: number, opts: { scenario: FakeOptions['scenario'], unlocked?: boolean }, body: (page: Page, fake: Fake) => Promise<void>) {
  const info = CHAPTERS.find(c => c.n === n)!
  test(`Chapter ${n}: ${info.title}`, async ({ page }) => {
    test.skip(!selected(n), 'not selected')
    startChapter(n, info.title)
    const theme = process.env.WALK_THEME === 'light' ? 'light' : 'dark'
    const fake = await installFakeBackend(page, { scenario: opts.scenario, unlocked: opts.unlocked ?? false, clock: true })
    if (theme === 'light') await page.addInitScript(`localStorage.setItem('nuxt-color-mode','light')`)
    await page.goto('/')
    await body(page, fake)
  })
}
