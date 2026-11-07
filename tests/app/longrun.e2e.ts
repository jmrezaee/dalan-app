import { execSync } from 'node:child_process'
import { appendFileSync, mkdirSync, writeFileSync } from 'node:fs'
import { addHost, button, byLabel, createVault, heading, jsClick, openHost, servers } from './support'

// `pnpm test:app:long` (test-plan 9.4, nightly): ten live sessions, each printing a line every second, for 30 minutes
// (LONG_RUN_MINUTES changes that). Memory of the app and its web view must not grow by 150 MB or more after the
// first two minutes of warming up. The RSS of every process of the app is sampled once a minute into longrun-rss.csv.
const MINUTES = Number(process.env.LONG_RUN_MINUTES ?? 30)
const SESSIONS = 10
const LIMIT_MB = 150
const OUT = '/work/test-results/app'

/** Resident memory, in MB, of the Dalan process and the WebKit processes it started. */
function rssMb(): number {
  const kb = execSync(`ps -eo rss=,comm= | grep -E 'dalan-app|WebKit' | awk '{s+=$1} END {print s+0}'`).toString().trim()
  return Number(kb) / 1024
}

describe('long run: ten sessions printing every second', function () {
  this.timeout((MINUTES + 20) * 60_000)

  it(`keeps memory growth under ${LIMIT_MB} MB over ${MINUTES} minutes`, async () => {
    const a = servers().a!
    mkdirSync(OUT, { recursive: true })
    await createVault()
    for (let i = 1; i <= SESSIONS; i++) await addHost({ address: a.net_host, port: a.net_port, user: a.user!, password: a.password, label: `long ${i}` })

    for (let i = 1; i <= SESSIONS; i++) {
      await jsClick($('//*[@role="tab"][contains(normalize-space(.),"Vault")]')) // the tab bar fills up: a pointer click can miss
      await openHost(`long ${i}`)
      if (i === 1) {
        await heading('First time connecting to long 1').waitForDisplayed()
        await button('Trust and connect').click()
      }
      await byLabel(`long ${i}: connected`).waitForDisplayed({ timeout: 60_000 })
      // Earlier terminals stay mounted but hidden: type into the one that is showing.
      await browser.waitUntil(() => browser.execute(() => {
        document.querySelectorAll('[data-lr]').forEach(x => x.removeAttribute('data-lr'))
        const t = Array.from(document.querySelectorAll<HTMLTextAreaElement>('.xterm-helper-textarea')).find(x => (x.closest('.xterm') as HTMLElement | null)?.offsetWidth)
        t?.setAttribute('data-lr', '1')
        return !!t
      }))
      await $('[data-lr="1"]').addValue(`while true; do echo "s${i} $(date +%T.%N) ${'x'.repeat(60)}"; sleep 1; done`)
      await browser.keys('Enter')
    }

    const csv = `${OUT}/longrun-rss.csv`
    writeFileSync(csv, 'minute,rss_mb\n')
    const samples: number[] = []
    for (let m = 0; m <= MINUTES; m++) {
      const mb = rssMb()
      samples.push(mb)
      appendFileSync(csv, `${m},${mb.toFixed(0)}\n`)
      console.log(`minute ${m}: ${mb.toFixed(0)} MB`)
      if (m < MINUTES) await browser.pause(60_000)
    }

    // Still alive and still printing: the newest line of the last session is from the last seconds.
    const lines = await browser.execute(() => Object.values((window as unknown as { __dalanTerm: Record<string, { buffer: { active: { length: number } } }> }).__dalanTerm).map(t => t.buffer.active.length))
    expect(lines.length).toBe(SESSIONS)
    expect(Math.min(...lines)).toBeGreaterThan(MINUTES * 30) // at least half the expected lines per session

    const warm = samples[Math.min(2, samples.length - 1)]!
    const growth = Math.max(...samples.slice(2)) - warm
    console.log(`growth after warm-up: ${growth.toFixed(0)} MB (limit ${LIMIT_MB})`)
    expect(growth).toBeLessThan(LIMIT_MB)
  })
})
