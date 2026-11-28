import { existsSync, readFileSync } from 'node:fs'
import { addHost, createVault, servers, type } from '../support'
import { addDb, answer, clickText, inTab, mysql, nav, openTransfer, psql, sh, tid, toTab, txt } from '../k-support'

// K9 (replica/testing/hand-checks.md): a team's round trip, with a local stand-in for the team's servers. The
// "bastion" container is the only way in: staging and production PostgreSQL and a MySQL staging database are reached
// through it (SSH, then the database), as a team reaches theirs. Throwaway containers and made-up data only.

/** A database tab through the bastion asks about the bastion's key once: trust it, then wait for the tree. */
async function openThrough(name: string) {
  await nav('Databases')
  const row = await tid(`database-row-${name}`)
  await row.waitForDisplayed()
  await row.$('td:first-child').click()
  await browser.execute((id: string) => (document.querySelector(`[data-testid="${id}"]`) as HTMLElement | null)?.focus(), `database-row-${name}`)
  await browser.keys('Enter')
  const tree = $(`[data-testid="sql-tab-${name}"] [role="tree"]`)
  await browser.waitUntil(async () => (await tree.isDisplayed().catch(() => false)) || (await $(`//*[@data-testid="sql-tab-${name}"]//button[normalize-space(.)="Trust and connect"]`).isDisplayed().catch(() => false)), { timeout: 60_000 })
  const t = await $(`//*[@data-testid="sql-tab-${name}"]//button[normalize-space(.)="Trust and connect"]`)
  if (await t.isDisplayed().catch(() => false)) await t.click()
  await tree.waitForDisplayed({ timeout: 60_000 })
}

describe('K9: a team round trip through a bastion', function () {
  this.timeout(900_000)
  // The shared test server goes back to having no client tools (other tests check the "not installed" message).
  after(() => { sh('docker exec dalan-test-ssh sh -c "apk del postgresql16-client mariadb-client >/dev/null 2>&1 || true"') })
  before(async () => {
    // Staging and production differ: production has a column staging lacks, and one order differs.
    for (const db of ['k9_staging', 'k9_prod', 'k9_restored', 'k9_masked', 'k9_from_my']) psql('shop', `drop database if exists ${db}`)
    for (const db of ['k9_staging', 'k9_prod']) {
      psql('shop', `create database ${db}`)
      sh(`docker exec -i dalan-test-pg psql -q -U app -d ${db} < /work/tests/fixtures/dumps/pg16.sql`)
    }
    psql('k9_prod', 'alter table public.customers add column vip boolean default false; update public.orders set total = total + 1 where id = 2')
    mysql('drop database if exists k9my; create database k9my')
    sh('docker exec -i dalan-test-mysql mysql -uroot -ptest-mysql k9my < /work/tests/fixtures/dumps/mysql84.sql 2>/dev/null')
    sh('docker exec dalan-test-ssh sh -c "rm -rf /tmp/k9 && mkdir -p /tmp/k9 && chown -R test /tmp/k9" 2>/dev/null || true')
    // Like a team's bastion, it has the database client tools (backups run there, next to the data).
    sh('docker exec dalan-test-ssh sh -c "command -v pg_dump >/dev/null || apk add --no-cache postgresql16-client mariadb-client >/dev/null"')
    await createVault()
    const a = servers().a!
    await addHost({ address: a.net_host, port: a.net_port, user: a.user!, password: a.password, label: 'bastion' })
    await addDb({ engine: 'PostgreSQL', name: 'staging', address: 'pg', port: 5432, database: 'k9_staging', user: 'app', password: 'test-pg', environment: 'Staging', through: 'bastion', open: false })
    await addDb({ engine: 'PostgreSQL', name: 'production', address: 'pg', port: 5432, database: 'k9_prod', user: 'app', password: 'test-pg', environment: 'Production', through: 'bastion', open: false })
    await addDb({ engine: 'MySQL', name: 'my staging', address: 'mysql', port: 3306, database: 'k9my', user: 'root', password: 'test-mysql', environment: 'Staging', through: 'bastion', open: false })
    await addDb({ engine: 'PostgreSQL', name: 'local', address: 'pg', port: 5432, database: 'shop', user: 'app', password: 'test-pg', open: false })
  })

  it('a backup of staging made through the bastion restores into a new local database', async () => {
    await openThrough('staging')
    await openTransfer('staging')
    answer('save', '/tmp/k9-staging.dump')
    await clickText('//*[@data-testid="sql-tab-staging"]//*[@aria-label="Backup"]', 'Back up…')
    await browser.waitUntil(async () => (await $('//*[@data-testid="sql-tab-staging"]//*[@data-part="read-check"]').isExisting()), { timeout: 180_000 })
    expect(await txt($('//*[@data-testid="sql-tab-staging"]//*[@data-part="read-check"]'))).toContain('Read back')
    await openThrough('local')
    await openTransfer('local')
    await inTab('local', 'sql-restore').click()
    answer('open', '/tmp/k9-staging.dump')
    await tid('restore-pick-file').click()
    await $('//table[@aria-label="Objects in the dump"]').waitForDisplayed({ timeout: 30_000 })
    await clickText('//*[@data-testid="restore-dialog"]', 'Next')
    await clickText('//*[@data-testid="restore-dialog"]', 'A new database on the same server')
    await type(tid('restore-new-name'), 'k9_restored')
    await tid('restore-next').click()
    await tid('restore-confirm').waitForClickable({ timeout: 30_000 })
    await tid('restore-confirm').click()
    await browser.waitUntil(async () => (await txt(tid('restore-result'))).includes('Restored into k9_restored'), { timeout: 180_000 })
    expect(psql('k9_restored', 'select count(*) from public.orders')).toBe(psql('k9_staging', 'select count(*) from public.orders'))
    await clickText('//*[@data-testid="restore-dialog"]', 'Close')
  })

  it('staging copied into a local database with masking: no real email or name arrives', async () => {
    // The target is one of the open tabs.
    await toTab('staging')
    const t = 'staging'
    await inTab(t, 'sql-open-copy').click()
    await $(`[data-testid="sql-tab-${t}"] [aria-label="Target database"]`).selectByVisibleText('local · PostgreSQL')
    await $(`[data-testid="sql-tab-${t}"] [aria-label="Copy public.customers"]`).click()
    await $(`[data-testid="sql-tab-${t}"] [aria-label="Copy public.orders"]`).click()
    await clickText(`//*[@data-testid="sql-tab-${t}"]`, 'Into a new database on that server')
    await type($(`[data-testid="sql-tab-${t}"] [aria-label="New database name"]`), 'k9_masked')
    await inTab(t, 'copy-review').click()
    await inTab(t, 'copy-mask-suggested').waitForClickable({ timeout: 30_000 })
    await inTab(t, 'copy-mask-suggested').click()
    await inTab(t, 'copy-run').waitForClickable({ timeout: 30_000 })
    await inTab(t, 'copy-run').click()
    await browser.waitUntil(async () => (await txt(inTab(t, 'copy-report'))).includes('Copied into k9_masked'), { timeout: 180_000 })
    const real = psql('k9_staging', "select string_agg(email || '|' || name, ',') from public.customers").split(',')
    const masked = psql('k9_masked', "select string_agg(coalesce(email, '') || '|' || coalesce(name, ''), ',') from public.customers")
    for (const r of real) {
      const [email, name] = r.split('|')
      expect(masked).not.toContain(email!)
      expect(masked).not.toContain(`|${name},`)
    }
    expect(psql('k9_masked', 'select count(*) from public.orders')).toBe(psql('k9_staging', 'select count(*) from public.orders'))
  })

  it('a MySQL schema copied into PostgreSQL arrives, with its columns reviewed', async () => {
    await openThrough('my staging')
    const t = 'my staging'
    await inTab(t, 'sql-open-copy').click()
    await $(`[data-testid="sql-tab-${t}"] [aria-label="Target database"]`).selectByVisibleText('local · PostgreSQL')
    await $(`[data-testid="sql-tab-${t}"] [aria-label="Copy k9my.customers"]`).click()
    await $(`[data-testid="sql-tab-${t}"] [aria-label="Copy k9my.orders"]`).click()
    await clickText(`//*[@data-testid="sql-tab-${t}"]`, 'Into a new database on that server')
    await type($(`[data-testid="sql-tab-${t}"] [aria-label="New database name"]`), 'k9_from_my')
    await inTab(t, 'copy-review').click()
    await inTab(t, 'copy-plan').waitForDisplayed({ timeout: 30_000 })
    expect(await txt(inTab(t, 'copy-plan'))).toMatch(/columns/)
    await inTab(t, 'copy-run').click()
    await browser.waitUntil(async () => (await txt(inTab(t, 'copy-report'))).includes('Copied into k9_from_my'), { timeout: 180_000 })
    expect(psql('k9_from_my', 'select count(*) from public.orders')).toBe(mysql('select count(*) from k9my.orders'))
  })

  it('staging compared with production (read only): the difference shows and the migration script is kept', async () => {
    await openThrough('production')
    await toTab('staging')
    const tab = '//*[@data-testid="sql-tab-staging"]'
    await $('[data-testid="sql-tab-staging"] [data-testid="sql-open-compare"]').click()
    const cmp = '[data-testid="sql-tab-staging"] [data-testid="compare-tab"]'
    await $(`${cmp} [aria-label="Source schema"]`).selectByVisibleText('public')
    await $(`${cmp} [aria-label="Target database"]`).selectByVisibleText('production')
    await $(`${cmp} [aria-label="Target schema"]`).selectByVisibleText('public')
    await inTab('staging', 'compare-run').click()
    await browser.waitUntil(async () => (await txt($(`${tab}//*[@data-part="summary"]`))).includes('differences'), { timeout: 60_000 })
    expect(await txt($(`${tab}//*[@data-testid="compare-tab"]`))).toContain('vip')
    // vip is only in production: a drop, unticked until chosen.
    await $(`${cmp} [aria-label="Include column vip"]`).click()
    answer('save', '/tmp/k9-migration.sql')
    await clickText(tab, 'Copy as migration…')
    await browser.waitUntil(() => existsSync('/tmp/k9-migration.sql'), { timeout: 30_000 })
    expect(readFileSync('/tmp/k9-migration.sql', 'utf8')).toMatch(/vip/i)
    // Production is read only: nothing there changed.
    expect(psql('k9_prod', "select count(*) from information_schema.columns where table_name = 'customers' and column_name = 'vip'")).toBe('1')

    // One table by key, staging against production: the changed order is the one difference.
    await clickText(tab, 'Data in a table')
    await $(`${cmp} [aria-label="Source table: schema"]`).selectByVisibleText('public')
    await type($(`${cmp} [aria-label="Source table: name"]`), 'orders')
    await $(`${cmp} [aria-label="Target table: database"]`).selectByVisibleText('production')
    await $(`${cmp} [aria-label="Target table: schema"]`).selectByVisibleText('public')
    await inTab('staging', 'data-columns').click()
    await inTab('staging', 'data-compare').waitForClickable()
    await inTab('staging', 'data-compare').click()
    await browser.waitUntil(async () => (await $(`${tab}//*[@data-part="data-counts"]`).isDisplayed()), { timeout: 120_000 })
    expect((await txt($(`${tab}//*[@data-part="data-counts"]`))).replace(/\s+/g, ' ')).toContain('Different 1')
  })

  it('a backup of staging scheduled into a folder on the bastion runs, keeps 2, reads back, and checks out', async () => {
    await toTab('staging')
    await openTransfer('staging')
    const b = '//*[@data-testid="sql-tab-staging"]//*[@aria-label="Backup"]'
    await clickText(b, 'Save as a job…')
    await type($(`${b}//*[@aria-label="Job name"]`), 'nightly')
    await $(`${b}//*[@data-testid="backup-save-job-host"]`).click()
    await tid('host-picker').$('select[aria-label="Host"]').selectByVisibleText('bastion')
    await clickText('//*[@data-testid="host-picker"]', 'Connect')
    const trust = $('//*[@data-testid="host-picker"]//button[normalize-space(.)="Trust and connect"]')
    const folder = tid('host-picker').$('[aria-label="Folder on the host"]')
    await browser.waitUntil(async () => (await trust.isExisting()) || (await folder.isExisting()), { timeout: 30_000 })
    if (await trust.isExisting()) await trust.click()
    await type(tid('host-picker').$('[aria-label="Folder on the host"]'), '/tmp/k9')
    await browser.keys('Enter')
    await browser.pause(800)
    await tid('host-picker-use').click()
    await browser.waitUntil(async () => (await txt($(b))).includes('Saved as the job “nightly”'), { timeout: 30_000 })
    await nav('Jobs')
    await $('//*[@data-testid="job-nightly"]//*[@data-testid="job-schedule"]').click()
    await clickText('//*[@data-testid="schedule-dialog"]', 'Cron')
    // "Nightly" in minutes: every minute stands in for every night.
    await type(tid('schedule-dialog').$('[aria-label="Cron expression"]'), '* * * * *')
    await type(tid('schedule-dialog').$('[aria-label="Number of backups to keep"]'), '2')
    await tid('schedule-save').click()
    const files = () => sh('docker exec dalan-test-ssh ls /tmp/k9').split('\n').filter(f => f.startsWith('k9_staging-nightly-'))
    await browser.waitUntil(async () => files().length >= 2, { timeout: 240_000, interval: 5000 })
    await browser.pause(70_000)
    expect(files().length).toBe(2)
    let row = ''
    await browser.waitUntil(async () => (row = await txt(tid('job-nightly'))).includes('· ok'), { timeout: 60_000, timeoutMsg: `the job row says: ${row}` }).catch((e) => { throw new Error(`${e}; the job row says: ${row}`) })
    // The run's log has the read-back line.
    await $('//*[@data-testid="job-nightly"]//button[normalize-space(.)="History"]').click()
    // A closed <details> hides its log from innerText: read the newest run's log itself.
    await browser.waitUntil(async () => /Read back/i.test(await browser.execute(() => document.querySelector('[data-testid="job-nightly"] details pre')?.textContent ?? '')), { timeout: 30_000 })
    // Turn the schedule off before checking, so no run lands while it is checked.
    await $('//*[@data-testid="job-nightly"]//*[@data-testid="job-schedule"]').click()
    await clickText('//*[@data-testid="schedule-dialog"]', 'Off')
    await tid('schedule-save').click()

    // Check a backup from the bastion's folder, restored on the local server.
    await openThrough('local')
    await openTransfer('local')
    await inTab('local', 'backup-check').click()
    await clickText('//*[@data-testid="backup-check-dialog"]', 'A file in a folder on a host')
    await tid('backup-check-run').click()
    await tid('host-picker').$('select[aria-label="Host"]').selectByVisibleText('bastion')
    await clickText('//*[@data-testid="host-picker"]', 'Connect')
    await type(tid('host-picker').$('[aria-label="Folder on the host"]'), '/tmp/k9')
    await browser.keys('Enter')
    await browser.pause(800)
    await $(`//*[@data-testid="host-picker"]//button[contains(normalize-space(.),"${files().sort().at(-1)}")]`).click()
    await tid('host-picker-use').click()
    await browser.waitUntil(async () => (await $('//*[@data-testid="backup-check-dialog"]//*[@data-part="verdict"]').isExisting()), { timeout: 180_000 })
    const verdict = await txt($('//*[@data-testid="backup-check-dialog"]//*[@data-part="verdict"]'))
    expect(verdict).toMatch(/Restored on|reads back/i)
    expect(psql('shop', "select count(*) from pg_database where datname like 'dalan_check_%'")).toBe('0')
  })
})
