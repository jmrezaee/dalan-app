// The real-app walkthrough's step helper: the same caption bar, screenshot and steps.jsonl as tests/walk/step.ts, so
// tests/walk/gallery.mjs builds the same gallery for `pnpm walk:app`.
import { appendFileSync, mkdirSync } from 'node:fs'

const OUT = '/work/walk-results'
let chapterNumber = 0
let chapterTitle = ''
let stepNumber = 0

export function startChapter(n: number, title: string) {
  chapterNumber = n
  chapterTitle = title
  stepNumber = 0
  mkdirSync(OUT, { recursive: true })
}

async function caption(text: string, state: 'run' | 'pass' | 'fail') {
  await browser.execute((cap, st) => {
    let host = document.getElementById('dalan-walk-host')
    if (!host) {
      host = document.createElement('div')
      host.id = 'dalan-walk-host'
      host.style.cssText = 'position:fixed;inset:0;pointer-events:none;z-index:2147483647'
      host.attachShadow({ mode: 'open' }).innerHTML = '<style>.bar{position:fixed;top:0;left:0;right:0;padding:6px 14px;font:600 13px system-ui,sans-serif;color:#fff}.run{background:#0b7a76}.pass{background:#1f7a3d}.fail{background:#b3261e}</style><div class="bar"></div>'
      document.documentElement.append(host)
    }
    const bar = host.shadowRoot!.querySelector('.bar') as HTMLElement
    bar.className = `bar ${st}`
    bar.textContent = cap
  }, text, state).catch(() => {})
}

export async function step(info: { id: string, flow?: string, screen?: string, caption: string }, body: () => Promise<void>) {
  const n = ++stepNumber
  const text = `REAL APP · Chapter ${chapterNumber} · ${chapterTitle} · step ${n}${info.flow ? ` · ${info.flow}` : ''} · ${info.caption}`
  const started = Date.now()
  let error = ''
  let original: unknown = null
  await caption(text, 'run')
  try {
    await body()
  }
  catch (e) {
    original = e
    // eslint-disable-next-line no-control-regex
    error = (e instanceof Error ? e.message : String(e)).replace(/\u001b\[[0-9;]*m/g, '').split('\n').filter(Boolean).slice(0, 4).join(' | ')
  }
  await caption(text, error ? 'fail' : 'pass')
  // The picture without the caption bar.
  await browser.execute(() => { const h = document.getElementById('dalan-walk-host'); if (h) h.style.display = 'none' })
  mkdirSync(OUT, { recursive: true })
  const file = `${info.id}.png`
  await browser.saveScreenshot(`${OUT}/${file}`).catch(() => {})
  await browser.execute(() => { const h = document.getElementById('dalan-walk-host'); if (h) h.style.display = '' })
  appendFileSync(`${OUT}/steps.jsonl`, `${JSON.stringify({ ...info, chapter: chapterNumber, chapterTitle, n, status: error ? 'fail' : 'pass', ms: Date.now() - started, error, shot: file, theme: 'real' })}\n`)
  if (error) throw original
}
