// `pnpm walk`: plays the whole story in a visible browser, then opens the gallery.
//   pnpm walk                  headed, 600 ms between actions, dark
//   pnpm walk --fast           100 ms
//   pnpm walk --headless       no window (what CI runs as walk:ci); still makes the gallery and the video
//   pnpm walk --theme light    the whole walk in light
//   pnpm walk --from F07       start at the chapter that covers F07 (or a chapter number)
//   pnpm walk --only F11,F12   just these chapters (IDs or numbers)
//   pnpm walk --pause          wait for Enter after each chapter
//   pnpm walk --update         accept the current screenshots as the new baselines (look at the diffs first)
import { spawnSync } from 'node:child_process'
import { existsSync, rmSync } from 'node:fs'
import { createInterface } from 'node:readline/promises'
import { join } from 'node:path'
import { CHAPTERS } from './chapters.mjs'

const args = process.argv.slice(2)
const flag = name => args.includes(`--${name}`)
const value = (name) => {
  const i = args.indexOf(`--${name}`)
  return i >= 0 ? args[i + 1] : undefined
}
const headless = flag('headless')
const env = {
  ...process.env,
  WALK_DELAY: String(flag('fast') ? 100 : headless ? 0 : 600),
  WALK_HEADED: headless ? '0' : '1',
  WALK_THEME: value('theme') === 'light' ? 'light' : 'dark',
  ...(value('from') ? { WALK_FROM: value('from') } : {}),
  ...(value('only') ? { WALK_ONLY: value('only') } : {}),
  ...(flag('update') ? { WALK_UPDATE: '1' } : {}),
}

const root = process.cwd()
rmSync(join(root, 'walk-results'), { recursive: true, force: true })
rmSync(join(root, 'walk-report'), { recursive: true, force: true })

const playwright = (extra = [], e = env) => spawnSync('pnpm', ['exec', 'playwright', 'test', '--project=walk', ...(headless ? [] : ['--headed']), ...extra], { stdio: 'inherit', env: e }).status ?? 1

let status = 0
if (flag('pause')) {
  // One chapter at a time, so the terminal can wait for you in between.
  const rl = createInterface({ input: process.stdin, output: process.stdout })
  for (const c of CHAPTERS) {
    status ||= playwright(['-g', `Chapter ${c.n}:`], { ...env, WALK_FROM: '', WALK_ONLY: String(c.n) })
    await rl.question(`Chapter ${c.n} (${c.title}) done. Press Enter for the next one... `)
  }
  rl.close()
}
else {
  status = playwright()
}

const gallery = spawnSync('node', ['tests/walk/gallery.mjs'], { stdio: 'inherit', env })
const index = join(root, 'walk-report', 'index.html')
if (gallery.status === 0 && existsSync(index) && !headless) {
  spawnSync(process.platform === 'darwin' ? 'open' : 'xdg-open', [index], { stdio: 'ignore' })
  const html = join(root, 'playwright-report', 'index.html')
  if (existsSync(html)) console.log(`The Playwright report with the video and trace: pnpm exec playwright show-report`)
}
console.log(`Gallery: ${index}`)
process.exit(status)
