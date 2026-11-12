// Builds what the download page (site/index.html) needs besides its own markup (replica/launch/landing-v2.md):
//   - site/img/<id>.webp (1280 wide) and <id>-640.webp (phones) from the visual-test baselines in
//     tests/visual/__screenshots__ (dark theme: fake backend, made-up hosts, no real names or keys), plus
//     site/img/shots.json with every size
//   - site/fonts/: Inter and JetBrains Mono, the same files the app bundles (latin subset)
//   - the page's token block (between `tokens:start` and `tokens:end`), copied from the dark theme in
//     replica/design/tokens.css, so the page never carries colours of its own
// Run it after `pnpm test:visual:update` or a token change:  pnpm site:shots
// The WebP conversion uses the Chromium that Playwright already installed (no image library to add).
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { chromium } from '@playwright/test'

const root = process.cwd()
const baselines = join(root, 'tests', 'visual', '__screenshots__')
const site = join(root, 'site')
const img = join(site, 'img')

/** page id -> baseline name (the file is `<name>-dark.png`). Spec: landing-v2.md, "Sections". */
const SHOTS = {
  hero: 'S18-connected',
  palette: 'S17-palette',
  hosts: 'S02-grid',
  hosteditor: 'S03-edit-jump-page',
  runon: 'S13-run-on-hosts',
  identity: 'S18-identity-changed',
  sftp: 'S21-sftp',
  tunnels: 'S09-tunnels',
  dropped: 'S18-dropped-card',
  sidepanel: 'S20-side-panel',
  unlock: 'S24-unlock',
}
const QUALITY = 0.85
const BUDGET = 2 * 1024 * 1024 // the whole site/ folder

// ---- fonts
mkdirSync(join(site, 'fonts'), { recursive: true })
copyFileSync(join(root, 'node_modules/@fontsource-variable/inter/files/inter-latin-wght-normal.woff2'), join(site, 'fonts/inter-latin-wght-normal.woff2'))
copyFileSync(join(root, 'node_modules/@fontsource-variable/jetbrains-mono/files/jetbrains-mono-latin-wght-normal.woff2'), join(site, 'fonts/jetbrains-mono-latin-wght-normal.woff2'))

// ---- tokens: the declarations of the dark theme (no shadcn aliases, no terminal palette)
const css = readFileSync(join(root, 'replica/design/tokens.css'), 'utf8')
const darkBlock = /\n\.dark \{([\s\S]*?)\n\}/.exec(css)?.[1]
const rootBlock = /^:root, \.light \{([\s\S]*?)\n\}/m.exec(css)?.[1]
if (!darkBlock || !rootBlock) throw new Error('replica/design/tokens.css has no :root and .dark blocks')
const keep = (block, test) => block.split('\n').filter(l => l.trim().startsWith('--') && test(l.trim())).map(l => `    ${l.trim()}`).join('\n')
const tokenCss = [
  keep(rootBlock, l => /^--(font-|radius|motion-|z-|layout-)/.test(l)),
  keep(darkBlock, l => !/^--(sc-|ansi-|terminal-)/.test(l)),
].join('\n')
const pagePath = join(site, 'index.html')
const page = readFileSync(pagePath, 'utf8')
const marked = /(\/\* tokens:start[^\n]*\*\/\n)[\s\S]*?(\n\s*\/\* tokens:end \*\/)/
if (!marked.test(page)) throw new Error('site/index.html has no tokens:start / tokens:end markers')
writeFileSync(pagePath, page.replace(marked, `$1  :root {\n${tokenCss}\n  }$2`))

// ---- screenshots
rmSync(img, { recursive: true, force: true })
mkdirSync(img, { recursive: true })
const browser = await chromium.launch()
const tab = await browser.newPage()
const sizes = {}
for (const [id, name] of Object.entries(SHOTS)) {
  const src = join(baselines, `${name}-dark.png`)
  if (!existsSync(src)) throw new Error(`No baseline ${name}-dark.png. Run pnpm test:visual:update first.`)
  const dataUrl = `data:image/png;base64,${readFileSync(src).toString('base64')}`
  const out = await tab.evaluate(async ([url, quality]) => {
    const im = new Image()
    im.src = url
    await im.decode()
    const make = async (w) => {
      const h = Math.round(im.naturalHeight * (w / im.naturalWidth))
      const c = document.createElement('canvas')
      c.width = w
      c.height = h
      const ctx = c.getContext('2d')
      ctx.imageSmoothingQuality = 'high'
      ctx.drawImage(im, 0, 0, w, h)
      const blob = await new Promise(r => c.toBlob(r, 'image/webp', quality))
      const bytes = new Uint8Array(await blob.arrayBuffer())
      let s = ''
      for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000))
      return { w, h, b64: btoa(s) }
    }
    return { full: await make(Math.min(1280, im.naturalWidth)), small: await make(640) }
  }, [dataUrl, QUALITY])
  writeFileSync(join(img, `${id}.webp`), Buffer.from(out.full.b64, 'base64'))
  writeFileSync(join(img, `${id}-640.webp`), Buffer.from(out.small.b64, 'base64'))
  sizes[id] = { source: name, w: out.full.w, h: out.full.h, w640: out.small.w, h640: out.small.h }
}
await browser.close()
writeFileSync(join(img, 'shots.json'), `${JSON.stringify(sizes, null, 2)}\n`)

// ---- budget
const total = (dir) => readdirSync(dir, { withFileTypes: true }).reduce((n, e) => n + (e.isDirectory() ? total(join(dir, e.name)) : statSync(join(dir, e.name)).size), 0)
const bytes = total(site)
console.log(`site-shots: ${Object.keys(SHOTS).length} screenshots, site/ is ${(bytes / 1024).toFixed(0)} KB (budget ${BUDGET / 1024} KB)`)
if (bytes > BUDGET) {
  console.error('site-shots: site/ is over its size budget.')
  process.exit(1)
}
