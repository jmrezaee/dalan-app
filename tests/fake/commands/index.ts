import type { Handlers } from '../types'
import { checksFake } from './checks'
import { compareFake } from './compare'
import { copy } from './copy'
import { data } from './data'
import { jobsFake } from './jobs'
import { databases } from './databases'
import { hosts } from './hosts'
import { keychain } from './keychain'
import { mongo } from './mongo'
import { redis } from './redis'
import { redisConsole } from './redisConsole'
import { redisPubSub } from './redisPubSub'
import { redisWrite } from './redisWrite'
import { restore } from './restore'
import { sessions } from './sessions'
import { sql } from './sql'
import { sftp } from './sftp'
import { vault } from './vault'

/**
 * Every Rust command, keyed by its generated name. The type is `Handlers`, so a command added to the Rust side makes
 * `pnpm typecheck` fail here until it has a fake.
 */
export const handlers: Handlers = { ...vault, ...hosts, ...keychain, ...data, ...sessions, ...sftp, ...redis, ...redisWrite, ...redisConsole, ...redisPubSub, ...databases, ...sql, ...mongo, ...restore, ...copy, ...jobsFake, ...compareFake, ...checksFake }
