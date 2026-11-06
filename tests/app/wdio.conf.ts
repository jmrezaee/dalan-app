// WebdriverIO against the real Dalan binary, through tauri-driver and WebKitWebDriver. Runs inside the Linux image
// (tests/app/Dockerfile); `pnpm test:app` starts it. See replica/testing/test-plan.md section 8.5.
import type { Options } from '@wdio/types'
import { spawn, type ChildProcess } from 'node:child_process'
import { mkdirSync, rmSync, writeFileSync } from 'node:fs'

let driver: ChildProcess | undefined
const BIN = process.env.DALAN_BIN ?? '/work/src-tauri/target/debug/dalan-app'
const DATA = process.env.XDG_DATA_HOME ?? '/tmp/dalan-xdg'

export const config: Options.Testrunner & { capabilities: unknown[] } = {
  runner: 'local',
  tsConfigPath: './tsconfig.json',
  specs: [process.env.WDIO_SPECS ?? './specs/**/*.e2e.ts'],
  maxInstances: 1,
  capabilities: [{ 'tauri:options': { application: BIN } }],
  hostname: '127.0.0.1',
  port: 4444,
  path: '/',
  logLevel: 'warn',
  reporters: ['spec'],
  framework: 'mocha',
  mochaOpts: { ui: 'bdd', timeout: 180_000 },
  waitforTimeout: 15_000,

  // tauri-driver speaks WebDriver and starts the app (WebKitWebDriver does the talking to the webview).
  onPrepare: () => {
    driver = spawn('tauri-driver', ['--native-driver', '/usr/bin/WebKitWebDriver'], { stdio: ['ignore', process.stdout, process.stderr] })
  },
  onComplete: () => { driver?.kill() },

  // A failed test leaves a picture and the page text, so the cause can be seen without a debugger.
  afterTest: async (test, _context, { passed }) => {
    if (passed) return
    mkdirSync('/work/test-results/app', { recursive: true })
    const name = test.title.replace(/[^a-z0-9]+/gi, '-').slice(0, 60)
    await browser.saveScreenshot(`/work/test-results/app/FAILED-${name}.png`).catch(() => {})
    const text = await browser.execute(() => document.body.innerText).catch(() => '')
    writeFileSync(`/work/test-results/app/FAILED-${name}.txt`, String(text))
  },

  // Every spec file starts the app on an empty data folder: a new vault, no hosts.
  beforeSession: () => { rmSync(DATA, { recursive: true, force: true }) },
}
