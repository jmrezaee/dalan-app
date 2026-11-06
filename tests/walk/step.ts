// The walkthrough's building blocks: `step()` shows a caption, runs the actions, takes a screenshot and records the
// result for the gallery. Actions go through `click`, `fill`, ... which outline the target and wait WALK_DELAY first.
import { test, type Locator, type Page } from '@playwright/test'
import { appendFileSync, mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

export interface StepInfo { id: string, flow?: string, screen?: string, caption: string }

const OUT = join(process.cwd(), 'walk-results')
const delay = () => Number(process.env.WALK_DELAY ?? 0)
const headed = () => process.env.WALK_HEADED === '1'

let chapterNumber = 0
let chapterTitle = ''
let stepNumber = 0
export function startChapter(n: number, title: string) {
  chapterNumber = n
  chapterTitle = title
  stepNumber = 0
  mkdirSync(OUT, { recursive: true })
}

const HOST = 'dalan-walk-host'

/** The caption bar and the outline live in their own shadow root, so the app never sees them. */
async function paint(page: Page, caption: string, state: 'run' | 'pass' | 'fail', box?: { x: number, y: number, width: number, height: number } | null) {
  await page.evaluate(([cap, st, b, id]) => {
    let host = document.getElementById(id as string)
    if (!host) {
      host = document.createElement('div')
      host.id = id as string
      host.style.cssText = 'position:fixed;inset:0;pointer-events:none;z-index:2147483647'
      host.attachShadow({ mode: 'open' }).innerHTML = '<style>.bar{position:fixed;top:0;left:0;right:0;padding:6px 14px;font:600 13px system-ui,sans-serif;color:#fff;display:flex;gap:8px;align-items:center}.run{background:#0b7a76}.pass{background:#1f7a3d}.fail{background:#b3261e}.box{position:fixed;border:2px solid #3cc8c0;border-radius:6px;box-shadow:0 0 0 4px rgba(60,200,192,.25)}</style><div class="bar"></div><div class="box" hidden></div>'
      document.documentElement.append(host)
    }
    const root = host.shadowRoot!
    const bar = root.querySelector('.bar') as HTMLElement
    bar.className = `bar ${st}`
    bar.textContent = cap as string
    const box = root.querySelector('.box') as HTMLElement
    const r = b as { x: number, y: number, width: number, height: number } | null
    if (r) Object.assign(box.style, { left: `${r.x - 3}px`, top: `${r.y - 3}px`, width: `${r.width + 6}px`, height: `${r.height + 6}px` })
    box.hidden = !r
  }, [caption, state, box ?? null, HOST] as const).catch(() => {})
}

const pause = (page: Page, ms = delay()) => (ms ? page.waitForTimeout(ms) : Promise.resolve())

async function outline(page: Page, target: Locator) {
  const box = await target.first().boundingBox({ timeout: 5000 }).catch(() => null)
  await paint(page, currentCaption, 'run', box)
  await pause(page)
}

// ---- actions: outline the target, wait, act
export const click = async (page: Page, l: Locator, o?: Parameters<Locator['click']>[0]) => { await outline(page, l); await l.click(o) }
export const dblclick = async (page: Page, l: Locator) => { await outline(page, l); await l.dblclick() }
export const fill = async (page: Page, l: Locator, text: string) => { await outline(page, l); await l.fill(text) }
export const press = async (page: Page, key: string) => { await pause(page); await page.keyboard.press(key) }
export const hover = async (page: Page, l: Locator) => { await outline(page, l); await l.hover() }

let currentCaption = ''

export async function step(page: Page, info: StepInfo, body: () => Promise<void>) {
  const n = ++stepNumber
  const caption = `Chapter ${chapterNumber} of 17 · ${chapterTitle} · step ${n}${info.flow ? ` · ${info.flow}` : ''}${info.screen ? ` ${info.screen}` : ''} · ${info.caption}`
  currentCaption = caption
  const started = Date.now()
  let error = ''
  let original: unknown = null
  await test.step(`${info.id} ${info.caption}`, async () => {
    await paint(page, caption, 'run')
    try {
      await body()
      await pause(page)
    }
    catch (e) {
      original = e
      // The first lines of the message, without colour codes: enough for the gallery to say what failed.
      // eslint-disable-next-line no-control-regex
      error = (e instanceof Error ? e.message : String(e)).replace(/\u001b\[[0-9;]*m/g, '').split('\n').filter(Boolean).slice(0, 4).join(' | ')
    }
    await paint(page, caption, error ? 'fail' : 'pass')
    // The caption and outline are hidden from the picture; the page itself is what the baseline compares.
    const shot = await page.screenshot({ animations: 'disabled', caret: 'hide', style: `#${HOST}{display:none!important}` }).catch(() => null)
    const file = `${info.id}.png`
    if (shot) writeFileSync(join(OUT, file), shot)
    appendFileSync(join(OUT, 'steps.jsonl'), `${JSON.stringify({ ...info, chapter: chapterNumber, chapterTitle, n, status: error ? 'fail' : 'pass', ms: Date.now() - started, error, shot: shot ? file : null, theme: process.env.WALK_THEME ?? 'dark' })}\n`)
    if (error && headed()) await page.waitForTimeout(3000)
    if (error) throw original
  })
}
