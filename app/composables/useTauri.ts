// The only place that talks to Rust. Stores call these wrappers, never `invoke` directly.
import { commands } from '~/types/bindings'
import type { AppError } from '~/types/bindings'

type Result<T> = { status: 'ok'; data: T } | { status: 'error'; error: AppError }

/** Thrown for any failed command. Carries the `{ code, message }` shape from Rust. */
export class CommandError extends Error {
  constructor(public readonly code: AppError['code'], message: string) {
    super(message)
    this.name = 'CommandError'
  }
}

/** Turns the generated `{ status, data | error }` result into a plain value or a thrown CommandError. */
export async function unwrap<T>(call: Promise<Result<T>>): Promise<T> {
  const res = await call
  if (res.status === 'error') throw new CommandError(res.error.code, res.error.message)
  return res.data
}

export function useTauri() {
  return {
    appInfo: () => unwrap(commands.appInfo()),
    vaultStatus: () => unwrap(commands.vaultStatus()),
    vaultCreate: (masterPassword: string) => unwrap(commands.vaultCreate(masterPassword)),
    vaultUnlock: (masterPassword: string) => unwrap(commands.vaultUnlock(masterPassword)),
    vaultLock: () => unwrap(commands.vaultLock()),
    vaultChangePassword: (oldPassword: string, newPassword: string) => unwrap(commands.vaultChangePassword(oldPassword, newPassword)),
    vaultTouch: () => commands.vaultTouch(),
  }
}
