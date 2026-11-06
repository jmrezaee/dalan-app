import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { Browser } from 'playwright'
import type { Server } from 'node:http'
import { launchBrowser, openApp, seen, serveApp } from './harness'

let browser: Browser
let server: Server
let url: string

beforeAll(async () => {
  ;({ server, url } = await serveApp())
  browser = await launchBrowser()
})
afterAll(async () => {
  await browser?.close()
  server?.close()
})

describe('import preview (F10)', () => {
  it('shows rows with notes, leaves duplicates unticked, and imports only what is ticked', async () => {
    const page = await openApp(browser, url, { unlocked: true })
    await page.getByRole('button', { name: 'More create options' }).click()
    await page.getByRole('menuitem', { name: /Import hosts/ }).click()
    await page.getByRole('button', { name: 'Choose file…' }).click()
    await seen(page.getByText('An Include line was not followed.'))
    await seen(page.getByText('ProxyCommand was ignored.'))
    await seen(page.getByText('via bastion'))
    // The one that already exists starts unticked.
    await seen(page.getByText('2 of 3 selected'))
    await seen(page.getByText('Already added'))
    expect(await page.getByRole('checkbox', { name: 'Import web-1' }).getAttribute('aria-checked')).toBe('false')
    // The filter narrows the list, and Select all then acts on what is shown.
    await page.getByRole('dialog').getByLabel('Search', { exact: true }).fill('web')
    await seen(page.getByRole('checkbox', { name: 'Import web-1' }))
    expect(await page.getByRole('checkbox', { name: 'Import api' }).count()).toBe(0)
    await page.getByRole('checkbox', { name: 'Select all' }).click()
    await seen(page.getByText('3 of 3 selected'))
    await page.getByRole('checkbox', { name: 'Select all' }).click()
    await seen(page.getByText('2 of 3 selected'))
    await page.getByRole('dialog').getByLabel('Search', { exact: true }).fill('')
    await page.getByRole('button', { name: 'Import 2 hosts' }).click()
    await expect.poll(() => page.evaluate(() => (window as any).__fake.state.calls.find((c: string) => c.startsWith('import:')))).toBe('import:2')
    await page.close()
  })
})
