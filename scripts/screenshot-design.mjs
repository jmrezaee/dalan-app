// Screenshots /design (dev server must be running on :4000). Usage: pnpm screenshot:design
import { chromium } from 'playwright'

// Uses the installed Chrome, so no browser download is needed.
const browser = await chromium.launch({ channel: 'chrome' })
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } })
await page.goto('http://127.0.0.1:4000/design', { waitUntil: 'networkidle' })
await page.screenshot({ path: 'replica/design/design-page.png', fullPage: true })
await browser.close()
