import { execSync } from 'node:child_process'
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { button, createVault, servers, type } from '../support'

// Hand checks K1 to K7 (replica/testing/hand-checks.md section K) on the real app, against the Docker test servers:
// restore, copies, jobs and masking, schedules and host folders, compare, backup check, bulk and MongoDB import.
// System dialogs answer from DALAN_TEST_PICKS (debug builds only: `pick_or` in commands/sql.rs). Results are checked
// in the servers themselves (docker exec).

/** What an element shows (WebKitWebDriver's getText leaves some text out). */
async function txt(el: ChainablePromiseElement | WebdriverIO.Element): Promise<string> {
  const e = await el
  await e.waitForExist()
  return browser.execute((x: HTMLElement) => x.innerText, e as unknown as HTMLElement)
}
const tid = (id: string) => $(`[data-testid="${id}"]`)
const inTab = (name: string, id: string) => $(`[data-testid="sql-tab-${name}"] [data-testid="${id}"]`)
const sh = (cmd: string) => execSync(cmd, { encoding: 'utf8' }).trim()
const psql = (db: string, sql: string) => sh(`docker exec dalan-test-pg psql -U app -d ${db} -tAc "${sql.replace(/"/g, '\\"')}"`)
const mysql = (sql: string) => sh(`docker exec dalan-test-mysql mysql -uroot -ptest-mysql -N -e "${sql.replace(/"/g, '\\"')}" 2>/dev/null`)
const mongo = (js: string) => sh(`docker exec dalan-test-mongo mongosh --quiet -u app -p test-mongo --authenticationDatabase admin --eval '${js}'`)
const PICKS = '/tmp/dalan-picks'
function answer(kind: 'open' | 'folder' | 'save', path: string) {
  mkdirSync(PICKS, { recursive: true })
  writeFileSync(`${PICKS}/${kind}`, path)
}
const FIX = '/work/tests/fixtures/dumps'

async function clickText(scope: string, text: string) {
  const el = await $(`${scope}//*[(self::button or @role="radio" or @role="tab" or self::a or self::label) and contains(normalize-space(.),"${text}")]`)
  await el.waitForClickable()
  await el.click()
}
async function vault() {
  const tab = await $('//*[@role="tab"][normalize-space(.)="Vault"]')
  if (await tab.isDisplayed()) await tab.click()
}
async function nav(section: string) {
  await vault()
  await $(`//nav[@aria-label="Vault"]//a[contains(normalize-space(.),"${section}")]`).click()
}

/** Saves a database with the form (TLS off) and opens it in a tab. */
async function addDb(d: { engine: 'PostgreSQL' | 'MySQL' | 'MongoDB', name: string, address: string, port: number, database: string, user: string, password: string, environment?: 'Staging' | 'Production' }) {
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
  await type($('#db-address'), d.address)
  await type($('#db-port'), String(d.port))
  if (await $('#db-database').isExisting()) await type($('#db-database'), d.database)
  if (d.engine === 'MongoDB') await type($('#db-auth'), 'admin')
  await type($('#db-user'), d.user)
  await type($('#db-pass'), d.password)
  await clickText('//*[@role="radiogroup"][@aria-label="TLS"]', 'Off')
  await clickText('//*[@data-testid="db-form-sheet"]', 'Save')
  await tid('db-form-sheet').waitForExist({ reverse: true })
  await openDb(d.name)
}
async function openDb(name: string) {
  await nav('Databases')
  const row = await tid(`database-row-${name}`)
  await row.waitForDisplayed()
  // A table row isn't clickable through WebDriver: click its first cell, then Enter opens it (the row has the focus).
  await row.$('td:first-child').click()
  await browser.execute((id: string) => (document.querySelector(`[data-testid="${id}"]`) as HTMLElement | null)?.focus(), `database-row-${name}`)
  await browser.keys('Enter')
  const tab = d => $(`[data-testid="sql-tab-${d}"], [data-testid="mongo-tab-${d}"]`)
  await (await tab(name)).waitForDisplayed({ timeout: 30_000 })
  await browser.waitUntil(async () => (await $(`[data-testid="sql-tab-${name}"] [role="tree"], [data-testid="mongo-tab-${name}"] [role="tree"]`).isDisplayed()), { timeout: 30_000 })
}
async function toTab(name: string) {
  await $(`//*[@role="tab"][.//text()[normalize-space(.)="${name}"]]`).click()
}

describe('hand checks K1 to K7 on the real app', () => {
  before(async () => {
    for (const db of ['k1_restored', 'k2_copy', 'k7_bulk']) psql('shop', `drop database if exists ${db}`)
    for (const db of ['k3', 'k4']) mysql(`drop database if exists ${db}`)
    psql('shop', 'drop schema if exists k6a cascade; drop schema if exists k6b cascade')
    // K6's schemas exist before the pg tab reads its tree.
    psql('shop', "create schema k6a; create schema k6b; create table k6a.t (id bigint primary key, name text not null, tier text default 'basic'); create index on k6a.t (tier); create table k6b.t (id int primary key, name text, legacy text); create view k6b.gone as select 1;")
    psql('shop', "create table k6a.big as select g as id, 'n'||g as name from generate_series(1,50000) g; alter table k6a.big add primary key (id); create table k6b.big as select * from k6a.big; alter table k6b.big add primary key (id); delete from k6b.big where id = 7; update k6b.big set name = 'x' where id = 40000; insert into k6b.big values (60000, 'extra');")
    await createVault()
  })

  it('K1: a pg_dump custom archive restores into a new database after showing what is inside', async () => {
    await addDb({ engine: 'PostgreSQL', name: 'pg', address: 'pg', port: 5432, database: 'shop', user: 'app', password: 'test-pg' })
    await inTab('pg', 'sql-open-transfer').click()
    await inTab('pg', 'sql-restore').click()
    answer('open', `${FIX}/pg16.custom`)
    await tid('restore-pick-file').click()
    await $('//table[@aria-label="Objects in the dump"]').waitForDisplayed({ timeout: 30_000 })
    const inside = await txt(tid('restore-dialog'))
    expect(inside).toContain('orders')
    await clickText('//*[@data-testid="restore-dialog"]', 'Next')
    await clickText('//*[@data-testid="restore-dialog"]', 'A new database on the same server')
    await type(tid('restore-new-name'), 'k1_restored')
    await tid('restore-next').click()
    await tid('restore-confirm').waitForClickable({ timeout: 30_000 })
    await tid('restore-confirm').click()
    await browser.waitUntil(async () => (await txt(tid('restore-result'))).includes('Restored into k1_restored'), { timeout: 120_000 })
    expect(psql('k1_restored', 'select count(*) from public.orders')).toBe('4')
    expect(psql('k1_restored', 'select public.order_count(1)')).toBe('2')
    // "Open k1_restored" saves it as a database and opens it.
    await clickText('//*[@data-testid="restore-dialog"]', 'Open k1_restored')
    await $('[data-testid="sql-tab-pg · k1_restored"] [role="tree"]').waitForDisplayed({ timeout: 30_000 })
  })

  it('K2: a copy into a new database on the same server brings keys, the view and the rows', async () => {
    const t = 'pg · k1_restored'
    await inTab(t, 'sql-open-copy').click()
    await $(`[data-testid="sql-tab-${t}"] [aria-label="Target database"]`).selectByVisibleText('pg')
    await $(`[data-testid="sql-tab-${t}"] [aria-label="Copy public.customers"]`).click()
    await $(`[data-testid="sql-tab-${t}"] [aria-label="Copy public.orders"]`).click()
    await $(`[data-testid="sql-tab-${t}"] [aria-label="Into a new database on that server"]`).click()
    await type($(`[data-testid="sql-tab-${t}"] [aria-label="New database name"]`), 'k2_copy')
    await inTab(t, 'copy-review').click()
    await inTab(t, 'copy-run').waitForClickable({ timeout: 30_000 })
    await inTab(t, 'copy-run').click()
    await browser.waitUntil(async () => (await txt(inTab(t, 'copy-report'))).includes('Copied into k2_copy'), { timeout: 120_000 })
    expect(psql('k2_copy', 'select count(*) from public.orders')).toBe('4')
    expect(psql('k2_copy', "select count(*) from pg_constraint where conrelid = 'public.orders'::regclass and contype in ('p','f')")).toBe('2')
  })

  it('K3: PostgreSQL to MySQL keeps keys and values, lossy columns are marked', async () => {
    await addDb({ engine: 'MySQL', name: 'mysql', address: 'mysql', port: 3306, database: 'shop', user: 'root', password: 'test-mysql' })
    const t = 'pg · k1_restored'
    await toTab(t)
    await clickText(`//*[@data-testid="sql-tab-${t}"]`, 'Done')
    await $(`[data-testid="sql-tab-${t}"] [aria-label="Target database"]`).selectByVisibleText('mysql · MySQL')
    // Still ticked from before: only the new name changes.
    await type($(`[data-testid="sql-tab-${t}"] [aria-label="New database name"]`), 'k3')
    await inTab(t, 'copy-review').click()
    await inTab(t, 'copy-plan').waitForDisplayed({ timeout: 30_000 })
    expect(await txt(inTab(t, 'copy-plan'))).toMatch(/lossy/i)
    await inTab(t, 'copy-run').click()
    await browser.waitUntil(async () => (await txt(inTab(t, 'copy-report'))).includes('Copied into k3'), { timeout: 120_000 })
    expect(mysql('select count(*) from k3.orders')).toBe('4')
    expect(mysql("select count(*) from information_schema.table_constraints where table_schema = 'k3' and constraint_type = 'FOREIGN KEY'")).toBe('1')
  })

  it('K4: a masked copy saved as a job runs again from Jobs and is recorded', async () => {
    const t = 'pg · k1_restored'
    await clickText(`//*[@data-testid="sql-tab-${t}"]`, 'Done')
    await $(`[data-testid="sql-tab-${t}"] [aria-label="Target database"]`).selectByVisibleText('mysql · MySQL')
    await $(`[data-testid="sql-tab-${t}"] [aria-label="Copy public.orders"]`).click()
    // Still ticked from before: only the new name changes.
    await type($(`[data-testid="sql-tab-${t}"] [aria-label="New database name"]`), 'k4')
    await inTab(t, 'copy-review').click()
    await inTab(t, 'copy-mask-suggested').waitForClickable({ timeout: 30_000 })
    await inTab(t, 'copy-mask-suggested').click()
    await clickText(`//*[@data-testid="sql-tab-${t}"]`, 'Save as a job…')
    await type($(`[data-testid="sql-tab-${t}"] [aria-label="Job name"]`), 'k4 copy')
    await inTab(t, 'copy-save-job').click()
    await inTab(t, 'copy-run').click()
    await browser.waitUntil(async () => (await txt(inTab(t, 'copy-report'))).includes('Copied into k4'), { timeout: 120_000 })
    const real = psql('k1_restored', 'select string_agg(email, \',\' order by id) from public.customers')
    const masked = mysql('select group_concat(email order by id) from k4.customers')
    expect(masked).not.toBe(real)
    expect(masked.split(',').every(e => e === 'NULL' || e.endsWith('@example.test'))).toBe(true)
    await nav('Jobs')
    await $('//*[contains(@data-testid,"job-k4 copy")]').waitForDisplayed()
    expect(await txt(tid('job-k4 copy'))).toContain('ok')
  })

  it('K6: two schemas compare, sync until equal; two tables compare by key and are made to match', async () => {
    await toTab('pg')
    const tab = '//*[@data-testid="sql-tab-pg"]'
    await $(`[data-testid="sql-tab-pg"] [data-testid="sql-open-compare"]`).click()
    await $(`[data-testid="sql-tab-pg"] [aria-label="Source schema"]`).selectByVisibleText('k6a')
    await $(`[data-testid="sql-tab-pg"] [aria-label="Target database"]`).selectByVisibleText('pg')
    await $(`[data-testid="sql-tab-pg"] [aria-label="Target schema"]`).selectByVisibleText('k6b')
    await inTab('pg', 'compare-run').click()
    await browser.waitUntil(async () => (await txt($(`${tab}//*[@data-part="summary"]`))).includes('differences'), { timeout: 30_000 })
    await $('[aria-label="Include column legacy"]').click()
    await $('[aria-label="Include view gone"]').click()
    await inTab('pg', 'compare-sync').click()
    await type($(`[data-testid="sql-tab-pg"] [aria-label="Type pg to confirm"]`), 'pg')
    await inTab('pg', 'compare-commit').click()
    await browser.waitUntil(async () => (await txt($(`${tab}//*[@data-part="report"]`))).includes('Ran'), { timeout: 30_000 })
    await inTab('pg', 'compare-run').click()
    await browser.waitUntil(async () => (await txt($(`${tab}//*[@data-part="summary"]`))).includes('No differences'), { timeout: 30_000 })

    await clickText(tab, 'Data in a table')
    await $(`[data-testid="sql-tab-pg"] [aria-label="Source table: schema"]`).selectByVisibleText('k6a')
    await type($(`[data-testid="sql-tab-pg"] [aria-label="Source table: name"]`), 'big')
    await $(`[data-testid="sql-tab-pg"] [aria-label="Target table: database"]`).selectByVisibleText('pg')
    await $(`[data-testid="sql-tab-pg"] [aria-label="Target table: schema"]`).selectByVisibleText('k6b')
    await inTab('pg', 'data-columns').click()
    await inTab('pg', 'data-compare').waitForClickable()
    await inTab('pg', 'data-compare').click()
    await browser.waitUntil(async () => (await $(`${tab}//*[@data-part="data-counts"]`).isDisplayed()), { timeout: 120_000 })
    const counts = await txt($(`${tab}//*[@data-part="data-counts"]`))
    expect(counts.replace(/\s+/g, ' ')).toContain('Only in source 1 Only in target 1 Different 1 Identical 49,998')
    await inTab('pg', 'data-match').click()
    await inTab('pg', 'data-match-commit').click()
    await browser.waitUntil(async () => (await $(`${tab}//*[@data-part="matched"]`).isDisplayed()), { timeout: 120_000 })
    expect(psql('shop', 'select count(*) from k6b.big')).toBe('50000')
    expect(psql('shop', "select name from k6b.big where id = 40000")).toBe('n40000')
  })

  it('K7: a backup reads back and checks out in a scratch database; bulk export and import; MongoDB import', async () => {
    await toTab('pg')
    await inTab('pg', 'sql-open-transfer').click()
    answer('save', '/tmp/k7-shop.dump')
    await clickText('//*[@data-testid="sql-tab-pg"]//*[@aria-label="Backup"]', 'Back up…')
    await browser.waitUntil(async () => (await $('//*[@data-testid="sql-tab-pg"]//*[@data-part="read-check"]').isExisting()), { timeout: 120_000 })
    expect(await txt($('//*[@data-testid="sql-tab-pg"]//*[@data-part="read-check"]'))).toContain('Read back')
    // A cut-off copy doesn't read back.
    writeFileSync('/tmp/k7-cut.dump', readFileSync('/tmp/k7-shop.dump').subarray(0, 2000))
    await inTab('pg', 'backup-check').click()
    await clickText('//*[@data-testid="backup-check-dialog"]', 'A file on this computer')
    answer('open', '/tmp/k7-cut.dump')
    await clickText('//*[@data-testid="backup-check-dialog"]', 'Choose a file…')
    await tid('backup-check-run').click()
    await browser.waitUntil(async () => (await $('//*[@data-testid="backup-check-dialog"]//*[@data-part="verdict"]').isExisting()), { timeout: 60_000 })
    expect(await txt($('//*[@data-testid="backup-check-dialog"]//*[@data-part="verdict"]'))).toContain('doesn\'t read back')
    await clickText('//*[@data-testid="backup-check-dialog"]', 'The backup just made')
    await tid('backup-check-run').click()
    await browser.waitUntil(async () => (await txt($('//*[@data-testid="backup-check-dialog"]//*[@data-part="verdict"]'))).includes('Restored on'), { timeout: 180_000 })
    expect(psql('shop', "select count(*) from pg_database where datname like 'dalan_check_%'")).toBe('0')
    await clickText('//*[@data-testid="backup-check-dialog"]', 'Close')

    // Bulk export of two tables into a folder, then back into a fresh database.
    sh('rm -rf /tmp/k7out && mkdir -p /tmp/k7out')
    answer('folder', '/tmp/k7out')
    const ex = '//*[@data-testid="sql-tab-pg"]//*[@data-testid="bulk-export"]'
    await $(`${ex}//label[contains(normalize-space(.),"k6a.big")]`).click()
    await $(`${ex}//label[contains(normalize-space(.),"k6a.t")]`).click()
    await $(`${ex}//*[@data-testid="bulk-export-run"]`).click()
    await browser.waitUntil(async () => (await $(`${ex}//*[@data-part="bulk-done"]`).isExisting()), { timeout: 120_000 })
    const folder = readdirSync('/tmp/k7out').find(n => n.startsWith('shop-export-'))!
    const files = readdirSync(`/tmp/k7out/${folder}`).sort()
    expect(files).toEqual(['k6a.big.csv', 'k6a.t.csv', 'manifest.json', 'schema.sql'])
    const manifest = JSON.parse(readFileSync(`/tmp/k7out/${folder}/manifest.json`, 'utf8'))
    expect(manifest.tables[0].rows).toBe(50000)

    psql('shop', 'create database k7_bulk')
    await addDb({ engine: 'PostgreSQL', name: 'k7', address: 'pg', port: 5432, database: 'k7_bulk', user: 'app', password: 'test-pg' })
    await inTab('k7', 'sql-open-transfer').click()
    answer('folder', `/tmp/k7out/${folder}`)
    const im = '//*[@data-testid="sql-tab-k7"]//*[@data-testid="bulk-import"]'
    await clickText(im, 'Choose a folder…')
    await $(`${im}//*[contains(@data-testid,"bulk-file-k6a.big.csv")]`).waitForDisplayed()
    // The schema k6a isn't in this database: the tables go into public, created from the manifest's types.
    await $(`${im}//*[@data-testid="bulk-import-review"]`).click()
    await $(`${im}//*[@data-testid="bulk-import-commit"]`).waitForClickable({ timeout: 60_000 })
    await $(`${im}//*[@data-testid="bulk-import-commit"]`).click()
    await browser.waitUntil(async () => (await $(`${im}//table[@aria-label="Import report"]`).isExisting()), { timeout: 120_000 })
    expect(psql('k7_bulk', 'select count(*) from public.big')).toBe('50000')

    // MongoDB: extended JSON with an ObjectId and a date, inserted; again as an upsert.
    mongo('db.getSiblingDB("k7").dropDatabase()')
    writeFileSync('/tmp/k7-items.jsonl', '{"_id": {"$oid": "5f1d7f3e9b1e8a3b2c4d5e6f"}, "sku": "a", "at": {"$date": "2026-01-01T00:00:00Z"}}\n{"sku": "b", "qty": 2}\n')
    await addDb({ engine: 'MongoDB', name: 'mongo', address: 'mongo', port: 27017, database: 'k7', user: 'app', password: 'test-mongo' })
    await $('[data-testid="mongo-tab-mongo"] [data-testid="mongo-open-transfer"]').click()
    answer('open', '/tmp/k7-items.jsonl')
    const mi = '//*[@data-testid="mongo-tab-mongo"]//*[@data-testid="mongo-import"]'
    await clickText(mi, 'Choose a file…')
    await $(`${mi}//*[@data-part="mg-preview"]`).waitForDisplayed()
    await $(`${mi}//select[@aria-label="Database to import into"]`).selectByVisibleText('k7').catch(() => {})
    await $(`${mi}//*[@data-testid="mongo-import-review"]`).click()
    await $(`${mi}//*[@data-testid="mongo-import-commit"]`).click()
    await browser.waitUntil(async () => (await $(`${mi}//*[@data-part="mg-report"]`).isExisting()), { timeout: 60_000 })
    expect(await txt($(`${mi}//*[@data-part="mg-report"]`))).toContain('inserted 2')
    expect(mongo('db.getSiblingDB("k7").items.findOne({sku: "a"}).at instanceof Date')).toBe('true')
    expect(mongo('db.getSiblingDB("k7").items.findOne({sku: "a"})._id instanceof ObjectId')).toBe('true')
  })

  it('K5: a backup job into a folder on host a runs on a schedule, keeps the last 2 and says what it missed while locked', async () => {
    const a = servers().a!
    sh('docker exec dalan-test-ssh sh -c "rm -rf /tmp/k5 && mkdir -p /tmp/k5 && chown -R test /tmp/k5" 2>/dev/null || true')
    await toTab('pg')
    await inTab('pg', 'sql-open-transfer').click()
    const b = '//*[@data-testid="sql-tab-pg"]//*[@aria-label="Backup"]'
    await clickText(b, 'Save as a job…')
    await type($(`${b}//*[@aria-label="Job name"]`), 'k5')
    // The host first: a saved host with its password, trusted in the picker.
    await nav('Hosts')
    await (await $('//button[contains(normalize-space(.),"Add host") or contains(normalize-space(.),"New host")]')).click()
    await type($('#host-address'), a.net_host)
    await type($('#host-port'), String(a.net_port))
    await type($('#host-label'), 'server a')
    await type($('#host-user'), a.user!)
    await type($('#host-password'), a.password!)
    await button('Save').click()
    await toTab('pg')
    await $(`${b}//*[@data-testid="backup-save-job-host"]`).click()
    await tid('host-picker').$('select[aria-label="Host"]').selectByVisibleText('server a')
    await clickText('//*[@data-testid="host-picker"]', 'Connect')
    const trust = await $('//*[@data-testid="host-picker"]//button[normalize-space(.)="Trust and connect"]')
    if (await trust.isExisting()) await trust.click()
    await type(tid('host-picker').$('[aria-label="Folder on the host"]'), '/tmp/k5')
    await browser.keys('Enter')
    await browser.pause(800)
    await tid('host-picker-use').click()
    await browser.waitUntil(async () => (await txt($(b))).includes('Saved as the job “k5”'), { timeout: 30_000 })
    await nav('Jobs')
    await $('//*[@data-testid="job-k5"]//*[@data-testid="job-schedule"]').click()
    await clickText('//*[@data-testid="schedule-dialog"]', 'Cron')
    await type(tid('schedule-dialog').$('[aria-label="Cron expression"]'), '* * * * *')
    await type(tid('schedule-dialog').$('[aria-label="Number of backups to keep"]'), '2')
    await tid('schedule-save').click()
    // Three runs, a minute apart: only the two newest of this job's files stay (and a file of our own is left alone).
    sh('docker exec dalan-test-ssh sh -c "touch /tmp/k5/notes.txt /tmp/k5/shop-other-2026-01-01-0000.dump"')
    const files = () => sh('docker exec dalan-test-ssh ls /tmp/k5').split('\n').filter(Boolean)
    await browser.waitUntil(async () => files().filter(f => f.startsWith('shop-k5-')).length >= 2, { timeout: 200_000, interval: 5000 })
    await browser.pause(70_000)
    const left = files()
    expect(left.filter(f => f.startsWith('shop-k5-')).length).toBe(2)
    expect(left).toContain('notes.txt')
    expect(left).toContain('shop-other-2026-01-01-0000.dump')
    expect(left.some(f => f.endsWith('.part'))).toBe(false)
    // Locked for two and a half minutes: at unlock it runs once and says what it missed.
    await vault()
    await $('//nav//button[contains(normalize-space(.),"Lock")] | //button[contains(normalize-space(.),"Lock")]').click()
    const confirm = await $('//button[normalize-space(.)="Lock anyway" or normalize-space(.)="Lock"]')
    if (await confirm.isExisting()) await confirm.click().catch(() => {})
    await browser.pause(150_000)
    await type($('#mp'), 'correct-horse-staple')
    await browser.keys('Enter')
    await $('nav[aria-label="Vault"]').waitForDisplayed({ timeout: 30_000 })
    await nav('Jobs')
    await browser.waitUntil(async () => (await $('//*[@data-testid="job-k5"]//*[@data-part="missed"]').isExisting()), { timeout: 90_000 })
    expect(await txt($('//*[@data-testid="job-k5"]//*[@data-part="missed"]'))).toMatch(/Missed \d+ runs? while closed or locked/)
    // A host used by a job can't be deleted.
    expect(existsSync('/tmp')).toBe(true)
  })
})
