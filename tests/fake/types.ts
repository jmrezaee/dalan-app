// Types for the fake backend. The command table is keyed by the generated bindings, so a new Rust command makes
// `pnpm typecheck` fail until it has a fake, and a changed Rust type breaks the fake that returns it.
import type { commands, ErrorCode } from '../../app/types/bindings'

type Cmds = typeof commands

/** Throws the one error shape every command uses. A plain function, so TypeScript narrows after `if (!x) fail(...)`. */
export function fail(code: ErrorCode, message: string): never {
  throw { code, message }
}

/** What the Rust command returns on success (the `data` of the generated result, or the plain value). */
type Data<R> = R extends { status: 'ok', data: infer D } ? D : R extends { status: 'error' } ? never : R
export type Out<K extends keyof Cmds> = Data<Awaited<ReturnType<Cmds[K]>>>

/** Arguments arrive keyed by the Rust parameter names in camelCase, as Tauri sends them. */
export type Args = Record<string, any>

export interface Ctx {
  m: import('./model').Model
  /** The Tauri channel id inside a channel argument. */
  channel: (v: unknown) => number
  /** Sends a typed message on a channel. */
  send: (channel: number, message: unknown) => void
  /** Sends raw terminal bytes on a data channel. */
  sendBytes: (channel: number, bytes: Uint8Array) => void
  /** Sends a Tauri event to the app (`tunnel-changed`, `vault-locked`, ...). */
  emit: (name: string, payload: unknown) => void
  /** Records a short call tag for tests, for example `reconnect:fake-1`. */
  tag: (s: string) => void
  /** A fresh id with a prefix, stable per run (no random ids in assertions). */
  id: (prefix: string) => string
}

export type Handler<K extends keyof Cmds> = (a: Args, c: Ctx) => Out<K> | Promise<Out<K>>
export type Handlers = { [K in keyof Cmds]: Handler<K> }
