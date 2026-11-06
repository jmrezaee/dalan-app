import { defineConfig, devices } from '@playwright/test'

// Browser tests (L6): the browser-test build of the SPA with a fake Rust backend. See replica/testing/test-plan.md.
// Playwright's own Chromium everywhere (not Google Chrome), so every machine renders the same.
export default defineConfig({
  testDir: 'tests',
  testMatch: '**/*.spec.ts',
  outputDir: 'test-results',
  fullyParallel: true,
  retries: process.env.CI ? 2 : 0,
  reporter: [['list'], ['html', { open: 'never', outputFolder: 'playwright-report' }]],
  use: {
    baseURL: 'http://127.0.0.1:4310',
    viewport: { width: 1280, height: 800 },
    trace: 'retain-on-failure',
    video: 'retain-on-failure',
    screenshot: 'only-on-failure',
    colorScheme: 'dark',
    locale: 'en-CA',
    timezoneId: 'America/Toronto',
  },
  webServer: {
    command: 'node tests/support/serve.mjs --port 4310',
    url: 'http://127.0.0.1:4310',
    reuseExistingServer: !process.env.CI,
    timeout: 180_000,
  },
  expect: { timeout: 10_000, toHaveScreenshot: { maxDiffPixelRatio: 0.01, animations: 'disabled', caret: 'hide' } },
  projects: [
    { name: 'e2e', testDir: 'tests/e2e', use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 } } },
    { name: 'fake', testDir: 'tests/fake', use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 } } },
  ],
})
