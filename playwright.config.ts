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
    { name: 'e2e', testDir: 'tests/e2e', grepInvert: /@perf/, use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 } } },
    // Timing tests (tagged @perf in tests/e2e/perf.spec.ts) need the machine to themselves: `pnpm test:perf`, one worker.
    { name: 'perf', testDir: 'tests/e2e', grep: /@perf/, fullyParallel: false, workers: 1, use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 }, video: 'off', trace: 'off' } },
    // The walkthrough: one chapter after another, in order, with a video and a trace. `pnpm walk` runs this with a window.
    { name: 'walk', testDir: 'tests/walk', fullyParallel: false, timeout: 15 * 60_000, workers: 1, use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 }, video: 'on', trace: 'on', actionTimeout: 8_000 } },
    { name: 'e2e-light', testDir: 'tests/e2e', grep: /@theme/, use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 }, colorScheme: 'light' } },
    { name: 'a11y', testDir: 'tests/a11y', use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 } } },
    { name: 'visual', testDir: 'tests/visual', snapshotPathTemplate: '{testDir}/__screenshots__/{arg}{ext}', use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 } } },
    { name: 'site', testDir: 'tests/site', use: { ...devices['Desktop Chrome'] } },
    { name: 'fake', testDir: 'tests/fake', use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 } } },
  ],
})
