import type { Handlers } from '../types'
import { data } from './data'
import { hosts } from './hosts'
import { keychain } from './keychain'
import { sessions } from './sessions'
import { sftp } from './sftp'
import { vault } from './vault'

/**
 * Every Rust command, keyed by its generated name. The type is `Handlers`, so a command added to the Rust side makes
 * `pnpm typecheck` fail here until it has a fake.
 */
export const handlers: Handlers = { ...vault, ...hosts, ...keychain, ...data, ...sessions, ...sftp }
