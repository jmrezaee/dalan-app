import type { Seed } from '../model'
import { dir, file, NOW } from './helpers'
import { team } from './team'

/** A new install: no vault yet. */
const fresh = (): Seed => ({ vault: { exists: false, unlocked: false }, now: NOW })
/** A vault with nothing in it, unlocked from the start by the test when it wants. */
const empty = (): Seed => ({ vault: { exists: true, unlocked: false }, now: NOW, files: { local: [dir('home', [dir('me', [file('notes.txt', 412), file('.profile', 807)])])] } })
/** Two hosts: what the first browser tests were written against. */
const basic = (): Seed => ({
  now: NOW,
  hosts: [
    { id: 'h1', label: 'web-1', address: '10.0.0.1', username: 'me', password: true },
    { id: 'h2', label: 'db-1', address: '10.0.0.2', port: 2222, username: 'me', password: true },
  ],
  files: { local: [dir('home', [dir('me', [file('notes.txt', 412), file('.profile', 807)])])] },
})

export const scenarios = { fresh, empty, basic, team } as const
export type ScenarioName = keyof typeof scenarios
export { NOW }
