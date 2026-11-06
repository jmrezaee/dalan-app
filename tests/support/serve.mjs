// Serves the browser-test build of the SPA (`DALAN_TEST=1 pnpm build` -> .output-test/public) for Playwright.
// Unknown paths get index.html, like Tauri does. The build runs only when it is missing or older than app/.
import { createServer } from 'node:http'
import { execSync } from 'node:child_process'
import { existsSync, readdirSync, statSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { extname, join, normalize } from 'node:path'

const ROOT = join(process.cwd(), '.output-test', 'public')
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.woff2': 'font/woff2', '.svg': 'image/svg+xml', '.png': 'image/png' }
const port = Number(process.argv[process.argv.indexOf('--port') + 1]) || 4310

/** Newest modified time under a folder (files only). */
function newest(dir) {
  let t = 0
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name)
    t = Math.max(t, e.isDirectory() ? newest(p) : statSync(p).mtimeMs)
  }
  return t
}

const index = join(ROOT, 'index.html')
if (!existsSync(index) || statSync(index).mtimeMs < newest('app')) {
  console.log('Building the browser-test app (DALAN_TEST=1)...')
  execSync('pnpm build', { stdio: 'inherit', env: { ...process.env, DALAN_TEST: '1' } })
}

createServer(async (req, res) => {
  const clean = normalize(decodeURIComponent((req.url ?? '/').split('?')[0])).replace(/^(\.\.[/\\])+/, '')
  let file = join(ROOT, clean)
  if (!existsSync(file) || clean.endsWith('/')) file = join(ROOT, 'index.html')
  try {
    res.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream' })
    res.end(await readFile(file))
  }
  catch {
    res.writeHead(404).end()
  }
}).listen(port, '127.0.0.1', () => console.log(`Serving ${ROOT} on http://127.0.0.1:${port}`))
