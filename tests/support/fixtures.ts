// The test fixture every browser spec uses: Playwright's `test`, plus a check after each test that no secret the tests
// typed in ever came back out of the fake backend (hard rule 1: secrets never reach JavaScript).
import { test as base, expect } from '@playwright/test'

/** Secret values the specs and the walk type into the app. If one shows up in an answer, a command leaked it. */
export const TEST_SECRETS = ['correct-horse', 'right-passphrase', 'a-test-password', 'sk-ant-test-key', 'hunter2-test-secret', 'pp-test-secret']

export const test = base.extend<{ noSecretsReturned: void }>({
  noSecretsReturned: [async ({ page }, use) => {
    await use()
    const answers = await page.evaluate(() => (window as any).__fake?.state?.answers as string[] | undefined).catch(() => undefined)
    for (const secret of TEST_SECRETS) {
      const leaked = (answers ?? []).filter(a => a.includes(secret))
      expect(leaked, `an answer carried the secret "${secret}"`).toEqual([])
    }
  }, { auto: true }],
})
export { expect }
