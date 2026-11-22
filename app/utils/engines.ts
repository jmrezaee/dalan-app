import type { Engine } from '~/types/bindings'

/** What the UI says about each engine: the two-letter badge (our own label, no vendor logos), its colour token and a note. */
export const ENGINES: Record<Engine, { label: string, short: string, color: string, note: string }> = {
  postgres: { label: 'PostgreSQL', short: 'PG', color: 'text-ansi-blue', note: '12 and later' },
  mysql: { label: 'MySQL', short: 'My', color: 'text-ansi-yellow', note: '5.7 and 8' },
  mariadb: { label: 'MariaDB', short: 'Ma', color: 'text-ansi-magenta', note: '10.5 and later' },
  mongodb: { label: 'MongoDB', short: 'Mg', color: 'text-ansi-green', note: '5.0 and later, replica sets, sharded' },
  redis: { label: 'Redis', short: 'Rd', color: 'text-ansi-red', note: 'single, Cluster, Sentinel' },
}

/** The order of the engine picker (S35 step 1). */
export const PICKER_ORDER: Engine[] = ['postgres', 'mysql', 'mariadb', 'mongodb', 'redis']

/** The usual port of each engine (MongoDB lists ports inside its address). */
export const DEFAULT_PORT: Record<Engine, number | null> = { postgres: 5432, mysql: 3306, mariadb: 3306, mongodb: null, redis: 6379 }
