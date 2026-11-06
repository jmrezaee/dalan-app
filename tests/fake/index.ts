// Node side of the fake backend: puts the typed fake into a Playwright page and gives tests a small API to drive it.
//
//   const fake = await installFakeBackend(page, { scenario: 'team', unlocked: true })
//   await page.goto('/')
//   const [id] = await fake.sessionIds()
//   await fake.session(id).connect()
import { build } from 'esbuild'
import { fileURLToPath } from 'node:url'
import type { Page } from '@playwright/test'
import type { Options } from './backend'
import type { Seed } from './model'
import { NOW } from './scenarios/helpers'

export { NOW }
export type { Options as FakeOptions, Seed }

let bundle: Promise<string> | undefined
/** The page-side code, bundled once per run. */
const pageCode = () => (bundle ??= build({
  entryPoints: [fileURLToPath(new URL('./entry.ts', import.meta.url))],
  bundle: true, write: false, format: 'iife', platform: 'browser', target: 'es2022', logLevel: 'silent',
}).then(r => r.outputFiles[0]!.text))

export interface InstallOptions extends Options {
  /** Freeze time at `NOW` (2026-11-06 15:00 UTC) so relative times and timers are the same every run. Default true. */
  clock?: boolean
}

type Evalable = <T>(fn: string, ...args: unknown[]) => Promise<T>

export class Fake {
  constructor(readonly page: Page) {}

  private run: Evalable = (fn, ...args) => this.page.evaluate(([f, a]) => {
    const fake = (window as any).__fake
    return fake[f as string](...(a as unknown[]))
  }, [fn, args] as const) as never

  sessionIds = () => this.run<string[]>('sessionIds')
  /** Everything the UI asked for, in order: command names and short tags such as `reconnect:fake-3`. */
  calls = async (name?: string) => {
    const all = await this.page.evaluate(() => (window as any).__fake.state.calls as string[])
    return name ? all.filter(c => c === name || c.startsWith(`${name}:`)) : all
  }
  responses = () => this.page.evaluate(() => (window as any).__fake.state.responses as { id: string, prompt: string, answer: any }[])
  written = () => this.page.evaluate(() => (window as any).__fake.state.written as { id: string, text: string }[])
  /** Commands the app called that the fake does not know. Always empty while the contract holds. */
  unhandled = () => this.page.evaluate(() => (window as any).__fake.state.unhandled as string[])
  commandNames = () => this.run<string[]>('commandNames')
  model = () => this.run<any>('model')

  dropAll = () => this.run<void>('dropAll')
  rotateHostKey = (hostId: string, o: { type?: string } = {}) => this.run<void>('rotateHostKey', hostId, o)
  failNext = (cmd: string, error: { code: string, message: string }) => this.run<void>('failNext', cmd, error)
  lock = () => this.run<void>('lock')

  session(id: string) {
    return {
      state: (state: 'connecting' | 'authenticating' | 'ready' | 'disconnected' | 'closed') => this.run<void>('event', id, { type: 'state', state }),
      event: (e: unknown) => this.run<void>('event', id, e),
      /** Raw terminal text; ANSI escapes work. */
      output: (text: string) => this.run<void>('output', id, text),
      runScript: (name: 'ubuntu-login' | 'nginx-status' | 'apt-update' | 'sudo-prompt' | 'vim' | 'flood' | 'error-output') => this.run<void>('runScript', id, name),
      /** Shows a question and returns at once, for steps that only look at it. */
      ask: (kind: unknown, promptId = 'p1') => this.run<void>('event', id, { type: 'prompt', prompt_id: promptId, kind }),
      /** Shows a question and resolves with the UI's answer. */
      prompt: (kind: unknown, promptId = 'p1') => this.run<{ accept: boolean, text: string | null, remember: boolean }>('prompt', id, promptId, kind),
      /** Connect with the host-key question when the key is new or changed. Resolves when the session is ready or refused. */
      connect: () => this.run<'ready' | 'refused' | 'changed'>('connect', id),
      error: (code: string, message: string) => this.run<void>('event', id, { type: 'error', code, message }),
    }
  }

  tunnel = (id: string) => ({ state: (state: 'stopped' | 'starting' | 'active' | 'reconnecting' | 'error', message?: string) => this.run<void>('tunnel', id, state, message ?? null) })

  manualTransfers = (on = true) => this.run<void>('manualTransfers', on)
  transfer(id?: string) {
    const target = async () => id ?? (await this.run<string[]>('transferIds'))[0]!
    return {
      event: async (e: unknown) => this.run<void>('transfer', await target(), e),
      progress: async (item: number, fraction: number, total: number) => this.run<void>('transfer', await target(), { type: 'progress', item, bytes: Math.round(total * fraction), total }),
      done: async () => this.run<void>('transfer', await target(), { type: 'done', cancelled: false }),
    }
  }
  editSaved = () => this.run<void>('editSaved')
  remoteChanged = (on = true) => this.run<void>('remoteChanged', on)
}

/** Installs the fake before the app loads. Call before `page.goto`. */
export async function installFakeBackend(page: Page, opts: InstallOptions): Promise<Fake> {
  const { clock = true, ...options } = opts
  if (clock) await page.clock.install({ time: NOW })
  await page.addInitScript(`window.__FAKE_OPTIONS__ = ${JSON.stringify(options)};`)
  await page.addInitScript(await pageCode())
  return new Fake(page)
}
