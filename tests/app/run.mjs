// `pnpm test:app` and `pnpm walk:app`: the real Dalan binary on Linux, in Docker, against the Docker test servers.
//   pnpm test:app                 the real-app specs (tests/app/specs)
//   pnpm walk:app                 the walkthrough on the real app, recorded as video (see walk.e2e.ts)
//   pnpm test:app --spec smoke    one spec file
import { spawnSync } from 'node:child_process'
import { mkdirSync, rmSync } from 'node:fs'
import { join } from 'node:path'

const args = process.argv.slice(2)
const root = process.cwd()
const run = (cmd, a, opts = {}) => spawnSync(cmd, a, { stdio: 'inherit', cwd: root, ...opts })
const must = (r, what) => { if (r.status !== 0) { console.error(`${what} failed`); process.exit(r.status ?? 1) } }

// The servers (and the keys and servers.json the specs read).
must(run('scripts/test-servers.sh', ['up']), 'Starting the test servers')

// The image: Rust, WebKitGTK, tauri-driver, Xvfb, ffmpeg. Cached after the first build.
must(run('docker', ['build', '-t', 'dalan-app-test', 'tests/app']), 'Building the test image')

const spec = args.includes('--spec') ? args[args.indexOf('--spec') + 1] : null
const walk = args.includes('--walk')
const specs = walk ? './walk.e2e.ts' : spec ? `./specs/${spec}.e2e.ts` : './specs/**/*.e2e.ts'
// Old pictures and videos from an earlier run would be mistaken for this run's.
rmSync(join(root, 'test-results', 'app'), { recursive: true, force: true })
mkdirSync(join(root, 'test-results', 'app'), { recursive: true })
if (walk) { rmSync(join(root, 'walk-results'), { recursive: true, force: true }); rmSync(join(root, 'walk-report'), { recursive: true, force: true }) }

// The runner joins the servers' network, so `bastion:2222`, `inner:2222` and the rest resolve. Build outputs and
// dependencies live in volumes: the first run compiles Rust for a few minutes, the next ones take seconds.
const r = run('docker', [
  'run', '--rm', '--network', 'dalan-test-net', '--ipc=host', '--cap-add', 'SYS_ADMIN',
  '-v', `${root}:/work`,
  '-v', '/var/run/docker.sock:/var/run/docker.sock',
  '-v', 'dalan-app-node-modules:/work/node_modules',
  '-v', 'dalan-app-cargo-target:/work/src-tauri/target',
  '-v', 'dalan-app-cargo-registry:/usr/local/cargo/registry',
  '-v', 'dalan-app-pnpm-store:/root/.local/share/pnpm/store',
  '-e', `WDIO_SPECS=${specs}`, '-e', 'CI=true',
  'dalan-app-test', 'tests/app/in-container.sh',
])
if (walk) run('node', ['tests/walk/gallery.mjs'])
console.log(`Results: ${join(root, 'test-results', 'app')}`)
process.exit(r.status ?? 1)
