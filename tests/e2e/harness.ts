import { expect, type Locator, type Page } from '@playwright/test'
import { installFakeBackend, type Fake, type FakeOptions } from '../fake'

/** Installs the fake backend on the runner's page and opens the app on the unlock screen (or the vault, if `unlocked`). */
export async function openApp(page: Page, opts: { unlocked?: boolean, scenario?: FakeOptions['scenario'], clock?: boolean } = {}): Promise<Fake> {
  const fake = await installFakeBackend(page, { scenario: opts.scenario ?? 'basic', unlocked: opts.unlocked ?? false, clock: opts.clock })
  await page.goto('/')
  return fake
}

export async function unlock(page: Page, password = 'correct-horse') {
  await page.getByLabel('Master password').fill(password)
  await page.getByRole('button', { name: 'Unlock' }).click()
}

/** The fake session ids in the order they were opened. */
export const sessionIds = (page: Page) => page.evaluate(() => (window as any).__fake.sessionIds() as string[])
export const fakeEvent = (page: Page, id: string, e: unknown) => page.evaluate(([i, ev]) => (window as any).__fake.event(i, ev), [id, e] as const)
export const fakeOutput = (page: Page, id: string, text: string) => page.evaluate(([i, t]) => (window as any).__fake.output(i, t), [id, text] as const)

// Short names over Playwright's retrying assertions (they wait up to the `expect` timeout in playwright.config.ts).
export const seen = (l: Locator) => expect(l.first()).toBeVisible()
export const gone = (l: Locator) => expect(l).toHaveCount(0)
export const focused = (l: Locator) => expect(l.first()).toBeFocused()
export const enabled = (l: Locator, on: boolean) => (on ? expect(l.first()).toBeEnabled() : expect(l.first()).toBeDisabled())
export const hasText = (l: Locator, text: string) => expect(l.first()).toContainText(text)
export const hasAttr = (l: Locator, name: string, value: string) => expect(l.first()).toHaveAttribute(name, value)
