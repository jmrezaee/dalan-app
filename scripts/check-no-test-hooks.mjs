// Fails when the production build (.output/public) contains a browser-test hook. Run after `pnpm build`.
// The hooks are switched on only by DALAN_TEST=1, which writes to .output-test instead (nuxt.config.ts).
import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

const ROOT = join(process.cwd(), '.output', 'public')
const FORBIDDEN = ['__dalanTerm']

if (!existsSync(ROOT)) {
  console.error('No production build found. Run `pnpm build` first.')
  process.exit(2)
}
const hits = []
const walk = (dir) => {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name)
    if (e.isDirectory()) walk(p)
    else if (/\.(js|html|css|json)$/.test(e.name)) {
      const text = readFileSync(p, 'utf8')
      for (const f of FORBIDDEN) if (text.includes(f)) hits.push(`${p} contains ${f}`)
    }
  }
}
walk(ROOT)
if (hits.length) {
  console.error(hits.join('\n'))
  process.exit(1)
}
console.log('Production build has no test hooks.')
