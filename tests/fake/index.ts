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
import type { SystemIntegration_Serialize } from '../../app/types/bindings'
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
  /** Every answer the fake gave, for the "no secret ever returned" check. */
  answers = () => this.page.evaluate(() => (window as any).__fake.state.answers as string[])
  /** Commands the app called that the fake does not know. Always empty while the contract holds. */
  unhandled = () => this.page.evaluate(() => (window as any).__fake.state.unhandled as string[])
  commandNames = () => this.run<string[]>('commandNames')
  model = () => this.run<any>('model')

  dropAll = () => this.run<void>('dropAll')
  rotateHostKey = (hostId: string, o: { type?: string } = {}) => this.run<void>('rotateHostKey', hostId, o)
  failNext = (cmd: string, error: { code: string, message: string }) => this.run<void>('failNext', cmd, error)
  lock = () => this.run<void>('lock')
  /** Redis: take over a connection's events (see tests/fake/commands/redis.ts). */
  redisManual = (on = true) => this.run<void>('redisManual', on)
  redisConnIds = () => this.run<string[]>('redisConnIds')
  redisDropKey = (name: string) => this.run<void>('redisDropKey', name)
  redisFill = (n: number) => this.run<void>('redisFill', n)
  redisAddSet = (name: string, members: number) => this.run<void>('redisAddSet', name, members)
  redisKey = (name: string) => this.run<{ ttl_ms: number | null, value: any } | null>('redisKey', name)
  redisInfoCalls = () => this.run<number>('redisInfoCalls')
  redisHistory = (dbId: string) => this.run<string[]>('redisHistory', dbId)
  redisPublish = (channel: string, payload: string, times = 1) => this.run<number>('redisPublish', channel, payload, times)
  redisSubNames = () => this.run<string[]>('redisSubNames')
  redisBulkSlow = (on = true) => this.run<void>('redisBulkSlow', on)
  /** D12: the next restore pick returns a `clean` dump, a `hostile` one, one whose tables `clash`, or a `slow` restore. */
  restoreNext = (kind: 'clean' | 'hostile' | 'clash' | 'slow') => this.run<void>('restoreNext', kind)
  redisKeyCount = () => this.run<number>('redisKeyCount')
  settings = () => this.run<Record<string, string>>('settings')
  redisDb = (name: string) => this.run<Record<string, unknown> | null>('redisDb', name)
  redisMode = (name: string) => this.run<string | null>('redisMode', name)
  redisOpenConnIds = () => this.run<string[]>('redisOpenConnIds')
  redisEvent = (connId: string, e: unknown) => this.run<void>('redisEvent', connId, e)

  session(id: string) {
    return {
      state: (state: 'connecting' | 'authenticating' | 'ready' | 'disconnected' | 'closed') => this.run<void>('event', id, { type: 'state', state }),
      event: (e: unknown) => this.run<void>('event', id, e),
      /** Raw terminal bytes, which need not be valid text (a split emoji, binary output). */
      outputBytes: (bytes: number[]) => this.run<void>('outputBytes', id, bytes),
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

  quitRequested = (liveSessions: number) => this.run<void>('quitRequested', liveSessions)
  scheduledRun = (name: string, status: 'ok' | 'failed', missed = 0) => this.run<void>('scheduledRun', name, status, missed)
  putRemoteFile = (hostId: string, path: string) => this.run<void>('putRemoteFile', hostId, path)
  receiveLink = (target: { user: string | null, host: string, port: number } | null) => this.run<void>('receiveLink', target)
  setSystem = (patch: Partial<SystemIntegration_Serialize>) => this.run<void>('setSystem', patch)
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
