import { execSync } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import { button, pick, type } from './support'

// Helpers for the K hand checks on the real app (specs/k-checks.e2e.ts, specs/k9-bastion.e2e.ts).

/** What an element shows (WebKitWebDriver's getText leaves some text out). */
export async function txt(el: ChainablePromiseElement | WebdriverIO.Element): Promise<string> {
  const e = await el
  await e.waitForExist()
  return browser.execute((x: HTMLElement) => x.innerText, e as unknown as HTMLElement)
}
export const tid = (id: string) => $(`[data-testid="${id}"]`)
export const inTab = (name: string, id: string) => $(`[data-testid="sql-tab-${name}"] [data-testid="${id}"]`)
export const sh = (cmd: string) => execSync(cmd, { encoding: 'utf8' }).trim()
export const psql = (db: string, sql: string) => sh(`docker exec dalan-test-pg psql -U app -d ${db} -tAc "${sql.replace(/"/g, '\\"')}"`)
export const mysql = (sql: string) => sh(`docker exec dalan-test-mysql mysql -uroot -ptest-mysql -N -e "${sql.replace(/"/g, '\\"')}" 2>/dev/null`)
export const mongo = (js: string) => sh(`docker exec dalan-test-mongo mongosh --quiet -u app -p test-mongo --authenticationDatabase admin --eval '${js}'`)
export const PICKS = '/tmp/dalan-picks'
export function answer(kind: 'open' | 'folder' | 'save', path: string) {
  mkdirSync(PICKS, { recursive: true })
  writeFileSync(`${PICKS}/${kind}`, path)
}
export const FIX = '/work/tests/fixtures/dumps'

export async function clickText(scope: string, text: string) {
  const el = await $(`(${scope}//*[(self::button or @role="radio" or @role="tab" or self::a or self::label) and contains(normalize-space(.),"${text}")])[last()]`)
  await el.waitForClickable()
  await el.click()
}
// A toast can sit over the tab strip or the sidebar; these clicks go to the element's own handler.
export async function jsClick(sel: string) {
  const el = await $(sel)
  await el.waitForExist()
  await browser.execute((e: HTMLElement) => e.click(), el as unknown as HTMLElement)
  await browser.pause(300)
}
export async function vault() {
  if (await $('//*[@role="tab"][normalize-space(.)="Vault"]').isDisplayed()) await jsClick('//*[@role="tab"][normalize-space(.)="Vault"]')
}
export async function nav(section: string) {
  await vault()
  await jsClick(`//nav[@aria-label="Vault"]//a[contains(normalize-space(.),"${section}")]`)
}

/** Saves a database with the form (TLS off) and opens it in a tab. */
export async function addDb(d: { engine: 'PostgreSQL' | 'MySQL' | 'MongoDB', name: string, address: string, port: number, database: string, user: string, password: string, environment?: 'Staging' | 'Production', through?: string, open?: boolean }) {
  await nav('Databases')
  await button('New database').click()
  await clickText('//*[@data-testid="new-database-dialog"]', d.engine)
  await clickText('//*[@data-testid="new-database-dialog"]', 'Continue')
  // The sheet slides in; until it stops, a click lands on the dialog closing behind it.
  await tid('db-form-sheet').waitForDisplayed()
  await tid('new-database-dialog').waitForExist({ reverse: true })
  await browser.pause(600)
  await type($('#db-name'), d.name)
  if (d.environment) await clickText('//*[@role="radiogroup"][@aria-label="Environment"]', d.environment)
  if (d.through) {
    await clickText('//*[@data-testid="db-form-sheet"]', 'Through a host')
    await pick($('#db-host'), d.through)
  }
  // MongoDB takes members as address:port.
  if (d.engine === 'MongoDB') {
    await type($('#db-address'), `${d.address}:${d.port}`)
  }
  else {
    await type($('#db-address'), d.address)
    await type($('#db-port'), String(d.port))
  }
  if (await $('#db-database').isExisting()) await type($('#db-database'), d.database)
  if (d.engine === 'MongoDB') await type($('#db-auth'), 'admin')
  await type($('#db-user'), d.user)
  await type($('#db-pass'), d.password)
  await clickText('//*[@role="radiogroup"][@aria-label="TLS"]', 'Off')
  await clickText('//*[@data-testid="db-form-sheet"]', 'Save')
  await tid('db-form-sheet').waitForExist({ reverse: true })
  if (d.open !== false) await openDb(d.name)
}
export async function openDb(name: string) {
  await nav('Databases')
  const row = await tid(`database-row-${name}`)
  await row.waitForDisplayed()
  // A table row isn't clickable through WebDriver: click its first cell, then Enter opens it (the row has the focus).
  await row.$('td:first-child').click()
  await browser.execute((id: string) => (document.querySelector(`[data-testid="${id}"]`) as HTMLElement | null)?.focus(), `database-row-${name}`)
  await browser.keys('Enter')
  const tab = (d: string) => $(`[data-testid="sql-tab-${d}"], [data-testid="mongo-tab-${d}"]`)
  await (await tab(name)).waitForDisplayed({ timeout: 30_000 })
  await browser.waitUntil(async () => (await $(`[data-testid="sql-tab-${name}"] [role="tree"], [data-testid="mongo-tab-${name}"] [role="tree"]`).isDisplayed()), { timeout: 30_000 })
}
export async function toTab(name: string) {
  await jsClick(`//*[@role="tab"][.//text()[normalize-space(.)="${name}"]]`)
}

// The Transfer tab's buttons move once the dump tool's version line arrives; wait for it before clicking them.
export async function openTransfer(tab: string) {
  await inTab(tab, 'sql-open-transfer').click()
  await browser.waitUntil(async () => /pg_dump|mysqldump|mariadb-dump|not found/i.test(await txt($(`[data-testid="sql-tab-${tab}"]`))), { timeout: 30_000 })
  await browser.pause(400)
}

