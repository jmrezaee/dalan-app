import { expect, test } from '../support/fixtures'
import type { Page } from '@playwright/test'
import type { Fake } from '../fake'
import { openApp } from './harness'

// R1 of Redis (replica/redis/spec.md): the Databases section (S26), the editor (S27), and a Redis tab's shell with its
// connection states (S33). F13: save a database, test it, open it in a tab. The fake backend plays the connection.
const nav = (page: Page) => page.getByRole('navigation', { name: 'Vault' })
const openDatabases = (page: Page) => nav(page).getByRole('link', { name: /Databases/ }).click()
const sheet = (page: Page) => page.getByRole('dialog', { name: /database/i })
const row = (page: Page, name: string) => page.getByRole('row', { name: new RegExp(name) })

test.describe('Databases (S26, S27)', () => {
  test('S26: the empty state invites the first database', async ({ page }) => {
    await openApp(page, { scenario: 'empty', unlocked: true })
    await openDatabases(page)
    await expect(page.getByRole('heading', { name: 'No databases yet' })).toBeVisible()
    await expect(page.getByText('Add one to browse its keys, run commands and watch channels.')).toBeVisible()
    await page.getByRole('button', { name: 'New database' }).click()
    await expect(sheet(page)).toBeVisible()
  })

  test('S26: the table shows name, environment, address, route, kind and write mode, and filters', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    await expect(nav(page).getByRole('link', { name: /Databases/ })).toContainText('4')
    await openDatabases(page)
    const prod = row(page, 'cache-prod')
    await expect(prod).toContainText('production')
    await expect(prod).toContainText('127.0.0.1:6379')
    await expect(prod).toContainText('db-1') // the host it goes through
    await expect(prod).toContainText('Single server')
    await expect(prod).toContainText('Confirm writes')
    await expect(row(page, 'local')).toContainText('Direct')
    await expect(row(page, 'local')).toContainText('Open')

    await page.getByPlaceholder('Filter databases').fill('staging')
    await expect(row(page, 'cache-staging')).toBeVisible()
    await expect(row(page, 'cache-prod')).toHaveCount(0)
    await page.getByPlaceholder('Filter databases').fill('nothing-like-this')
    await expect(page.getByText('No database matches nothing-like-this.')).toBeVisible()
  })

  test('F13, S27: a new database is checked, tested and saved; its password never comes back', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    await openDatabases(page)
    await page.getByRole('button', { name: 'New database' }).click()
    const s = sheet(page)
    // New ones start as "Confirm writes", reached directly.
    await expect(s.getByRole('radio', { name: /Confirm writes/ })).toHaveAttribute('aria-checked', 'true')
    await expect(s.getByRole('radio', { name: 'Directly' })).toHaveAttribute('aria-checked', 'true')
    // Cluster and Sentinel are not built yet: they can't be picked.
    await expect(s.getByRole('radio', { name: 'Cluster' })).toBeDisabled()

    // Saving with nothing filled in says what is missing, in the form.
    await s.getByRole('button', { name: 'Save' }).click()
    await expect(s.getByRole('alert')).toContainText('Give the database a name')

    await s.getByLabel('Name', { exact: true }).fill('queue-prod')
    await s.getByRole('radio', { name: 'Production' }).click()
    await s.getByRole('radio', { name: 'Through a host' }).click()
    await expect(s.getByLabel('Address, as seen from the host')).toHaveValue('127.0.0.1')
    await s.getByLabel('Host', { exact: true }).click()
    await page.getByRole('option', { name: 'bastion' }).click()
    await expect(s.getByText('Dalan signs in to bastion and talks to Redis from there.')).toBeVisible()
    await s.getByLabel('Username (optional)').fill('app-rw')
    await s.locator('#rdb-pass').fill('a-test-password')
    await s.getByRole('switch', { name: 'Use TLS' }).click()
    await expect(s.getByLabel('Certificate name (optional)')).toBeVisible()
    await s.getByRole('radio', { name: /Read-only/ }).click()

    await s.getByRole('button', { name: 'Test connection' }).click()
    await expect(s.getByRole('status')).toHaveText('Connected · Redis 7.2.4 · 3 ms')

    await s.getByRole('button', { name: 'Save' }).click()
    await expect(s).toHaveCount(0)
    const saved = row(page, 'queue-prod')
    await expect(saved).toContainText('production')
    await expect(saved).toContainText('bastion')
    await expect(saved).toContainText('Read-only')
    // The password went to Rust once; no answer ever carries it.
    expect((await fake.answers()).join('\n')).not.toContain('a-test-password')
    const stored = (await fake.model()).redisDbs.find((d: any) => d.name === 'queue-prod')
    expect(stored).toMatchObject({ password: true, username: 'app-rw', tls: true, write_mode: 'read_only', route: 'host' })
  })

  test('S27: a failed test says why in one line; editing shows a saved password as saved', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    await openDatabases(page)
    await row(page, 'cache-prod').click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Edit' }).click()
    const s = sheet(page)
    await expect(s.getByRole('heading', { name: 'Edit database' })).toBeVisible()
    await expect(s.getByLabel('Name', { exact: true })).toHaveValue('cache-prod')
    await expect(s.getByText('Remove saved password')).toBeVisible()
    await fake.failNext('redis_db_test', { code: 'redis_unreachable', message: 'Can\'t reach Redis from db-1. Nothing answered at 127.0.0.1:6379.' })
    await s.getByRole('button', { name: 'Test connection' }).click()
    await expect(s.getByRole('status')).toHaveText('Can\'t reach Redis from db-1. Nothing answered at 127.0.0.1:6379.')
    await page.keyboard.press('Escape')
    await expect(s).toHaveCount(0)
  })

  test('S26: deleting asks first, with Cancel as the way out', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    await openDatabases(page)
    await row(page, 'local').click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Delete' }).click()
    const dialog = page.getByRole('alertdialog', { name: 'Delete local?' })
    await dialog.getByRole('button', { name: 'Cancel' }).click()
    await expect(row(page, 'local')).toBeVisible()
    expect(await fake.calls('redis_db_delete')).toEqual([])

    await row(page, 'local').click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Delete' }).click()
    await dialog.getByRole('button', { name: 'Delete database' }).click()
    await expect(row(page, 'local')).toHaveCount(0)
  })
})

/** Opens a database from the table and returns its tab's panel. */
async function openTab(page: Page, name: string) {
  await openDatabases(page)
  await row(page, name).dblclick()
  return page.getByTestId(`redis-tab-${name}`)
}
const lastConn = async (fake: Fake) => (await fake.redisConnIds()).at(-1)!

test.describe('Redis tab (S28 shell, S33 states)', () => {
  test('F13: opening a database adds a tab, shows the steps and ends connected, with its toolbar', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    await fake.redisManual()
    const tab = await openTab(page, 'cache-prod')
    await expect(page.getByRole('tab', { name: 'cache-prod' })).toHaveAttribute('aria-selected', 'true')
    const conn = await lastConn(fake)

    // The plan arrives first: every step is listed before anything is tried.
    await fake.redisEvent(conn, { type: 'plan', steps: [{ label: 'Sign in to bastion', detail: null }, { label: 'Sign in to db-1', detail: 'through bastion' }, { label: 'Reach Redis at 127.0.0.1:6379', detail: null }, { label: 'Sign in as app-ro', detail: null }] })
    const steps = tab.getByTestId('redis-steps')
    await expect(steps.getByRole('heading', { name: 'Opening cache-prod' })).toBeVisible()
    await expect(steps.getByRole('listitem')).toHaveCount(4)
    await fake.redisEvent(conn, { type: 'step', index: 0, state: 'done', detail: null })
    await fake.redisEvent(conn, { type: 'step', index: 1, state: 'active', detail: 'Signing in as deploy.' })
    await expect(steps.getByRole('listitem').nth(0)).toHaveAttribute('data-state', 'done')
    await expect(steps.getByRole('listitem').nth(1)).toHaveAttribute('data-state', 'active')
    await expect(steps.getByRole('listitem').nth(1)).toContainText('Signing in as deploy.')
    await expect(steps.getByRole('listitem').nth(3)).toHaveAttribute('data-state', 'pending')

    await fake.redisEvent(conn, { type: 'ready', info: { version: '7.2.4', mode: 'standalone', replica: false, resp3: true, db_count: 16, databases: [{ index: 0, keys: 12408 }], selected: 0, latency_ms: 3 } })
    await expect(tab.getByRole('heading', { name: 'Pick a key to see its value.' })).toBeVisible()
    // The toolbar: name, the production chip, the route, the database picker, the four views, latency, the mode.
    const bar = tab.getByRole('banner')
    await expect(bar.getByRole('heading', { name: 'cache-prod' })).toBeVisible()
    await expect(bar.getByText('production', { exact: true })).toBeVisible()
    await expect(bar).toContainText('db-1')
    await expect(bar.getByRole('combobox', { name: 'Logical database' })).toContainText('db0 · 12,408 keys')
    await expect(bar.getByRole('tab')).toHaveText(['Browser', 'Console', 'Pub/Sub', 'Server'])
    await expect(bar).toContainText('3 ms')
    await expect(bar.getByRole('button', { name: 'Confirm writes' })).toBeVisible()
  })

  test('S33: a host question is asked in the tab and the answer goes back', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    await fake.redisManual()
    const tab = await openTab(page, 'cache-prod')
    const conn = await lastConn(fake)
    await fake.redisEvent(conn, { type: 'plan', steps: [{ label: 'Sign in to db-1', detail: null }, { label: 'Reach Redis at 127.0.0.1:6379', detail: null }, { label: 'Sign in as app-ro', detail: null }] })
    await fake.redisEvent(conn, { type: 'host', event: { type: 'prompt', prompt_id: 'p1', kind: { type: 'password', host: 'db-1', user: 'deploy' } } })
    const ask = tab.getByRole('alertdialog', { name: 'db-1 needs an answer' })
    await expect(ask).toContainText('db-1 needs an answer before Dalan can reach Redis.')
    await ask.locator('input[type="password"]').fill('host-password')
    await ask.getByRole('button', { name: 'Continue' }).click()
    await expect(ask).toHaveCount(0)
    const answer = (await fake.responses()).find(r => r.id === conn)
    expect(answer).toMatchObject({ prompt: 'p1', answer: { accept: true, text: 'host-password' } })
  })

  test('S33: failures say what happened in one line, with one useful action', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    await fake.redisManual()
    const tab = await openTab(page, 'cache-prod')
    await fake.redisEvent(await lastConn(fake), { type: 'failed', code: 'redis_auth_failed', message: 'Wrong username or password. Redis refused the sign-in.' })
    const failed = tab.getByTestId('redis-failed')
    await expect(failed).toContainText('Wrong username or password')
    await expect(failed).toContainText('Redis refused the sign-in.')
    await failed.getByRole('button', { name: 'Edit database' }).click()
    await expect(sheet(page).getByLabel('Name', { exact: true })).toHaveValue('cache-prod')
    await page.keyboard.press('Escape')

    // "Try again" is a fresh attempt: the old connection is closed and a new one opened.
    await page.getByRole('tab', { name: 'cache-prod' }).click()
    const before = (await fake.redisConnIds()).length
    await tab.getByRole('button', { name: 'Edit database' }).waitFor()
    await page.evaluate(() => undefined)
    const first = await lastConn(fake)
    await fake.redisEvent(first, { type: 'failed', code: 'redis_unreachable', message: 'Can\'t reach Redis from db-1. Nothing answered at 127.0.0.1:6379.' })
    await expect(failed).toContainText('Can\'t reach Redis')
    await failed.getByRole('button', { name: 'Try again' }).click()
    await expect.poll(async () => (await fake.redisConnIds()).length).toBe(before + 1)
    expect(await fake.redisOpenConnIds()).toEqual([await lastConn(fake)])
  })

  test('S33: a dropped connection says when it tries again, and "Reconnect now" starts over', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    const tab = await openTab(page, 'local') // connects by itself
    await expect(tab.getByRole('heading', { name: 'Pick a key to see its value.' })).toBeVisible()
    const conn = await lastConn(fake)
    await fake.redisEvent(conn, { type: 'dropped', attempt: 2, of: 5, retry_in_secs: 4 })
    const dropped = tab.getByTestId('redis-dropped')
    await expect(dropped).toContainText('The connection dropped')
    await expect(dropped).toContainText('Trying again in 4 s (2 of 5).')
    await dropped.getByRole('button', { name: 'Reconnect now' }).click()
    await expect(tab.getByRole('heading', { name: 'Pick a key to see its value.' })).toBeVisible()
    expect(await lastConn(fake)).not.toBe(conn)
  })

  test('a replica says it only accepts reads; the database picker switches databases', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    await fake.redisManual()
    const tab = await openTab(page, 'local')
    await fake.redisEvent(await lastConn(fake), { type: 'ready', info: { version: '6.2.14', mode: 'standalone', replica: true, resp3: false, db_count: 16, databases: [{ index: 0, keys: 12408 }, { index: 3, keys: 2 }], selected: 0, latency_ms: 140 } })
    await expect(tab.getByRole('status').filter({ hasText: 'replica' })).toContainText('This server is a replica. It only accepts reads.')
    await tab.getByRole('combobox', { name: 'Logical database' }).click()
    await page.getByRole('option', { name: 'db3 · 2 keys' }).click()
    await expect(tab.getByRole('combobox', { name: 'Logical database' })).toContainText('db3 · 2 keys')
    expect(await fake.calls('redis_select')).toHaveLength(1)
  })

  test('one tab per database; a production tab is marked; closing the tab closes its connection', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    await openTab(page, 'cache-prod')
    await page.getByRole('tab', { name: 'Vault' }).click()
    await row(page, 'cache-prod').dblclick() // again: the same tab comes forward
    await expect(page.getByRole('tab', { name: 'cache-prod' })).toHaveCount(1)
    expect(await fake.redisConnIds()).toHaveLength(1)
    await expect(page.getByTestId('redis-tab-cache-prod').getByRole('heading', { name: 'Pick a key to see its value.' })).toBeVisible()

    // The Redis tab is still mounted while another tab shows (its connection carries on).
    await page.getByRole('tab', { name: 'Vault' }).click()
    await expect(page.getByTestId('redis-tab-cache-prod')).toBeAttached()
    await expect(page.getByTestId('redis-tab-cache-prod')).toBeHidden()

    await page.getByRole('button', { name: 'Close cache-prod' }).click()
    await expect(page.getByRole('tab', { name: 'cache-prod' })).toHaveCount(0)
    expect(await fake.redisOpenConnIds()).toEqual([])
  })

  test('F10: locking the vault closes every Redis tab', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    await openTab(page, 'local')
    await expect(page.getByTestId('redis-tab-local').getByRole('heading', { name: 'Pick a key to see its value.' })).toBeVisible()
    await page.getByRole('tab', { name: 'Vault' }).click()
    await page.getByRole('button', { name: 'Lock', exact: true }).click()
    await expect(page.getByLabel('Master password')).toBeVisible()
    await page.getByLabel('Master password').fill('correct-horse')
    await page.getByRole('button', { name: 'Unlock' }).click()
    await expect(page.getByRole('tab', { name: 'local', exact: true })).toHaveCount(0)
    expect(await fake.redisOpenConnIds()).toEqual([])
  })
})

test.describe('ways in (S02, S17)', () => {
  test('S02: a host\'s menu opens a new database with that host as the route, or shows its saved ones', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    // The roaming laptop has no database yet: the editor opens with the route filled in.
    await page.getByText('roaming laptop', { exact: true }).first().click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Open Redis on this host…' }).click()
    const s = sheet(page)
    await expect(s.getByRole('radio', { name: 'Through a host' })).toHaveAttribute('aria-checked', 'true')
    await expect(s.getByLabel('Host', { exact: true })).toContainText('roaming laptop')
    await page.keyboard.press('Escape')

    // The bastion is the route to sessions-prod: its databases are listed instead.
    await page.getByRole('navigation', { name: 'Vault' }).getByRole('link', { name: /Hosts/ }).click()
    await page.getByText('bastion', { exact: true }).first().click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Open Redis on this host…' }).click()
    await expect(page.getByRole('heading', { name: 'Databases' })).toBeVisible()
    await expect(row(page, 'sessions-prod')).toBeVisible()
    await expect(row(page, 'local')).toHaveCount(0)
  })

  test('S17: the palette opens a saved database and offers "New database"', async ({ page }) => {
    await openApp(page, { scenario: 'team', unlocked: true })
    await expect(page.getByText('bastion', { exact: true }).first()).toBeVisible()
    await page.keyboard.press('ControlOrMeta+k')
    await page.getByPlaceholder('Search hosts, tabs and actions').fill('cache-staging')
    await page.keyboard.press('Enter')
    await expect(page.getByRole('tab', { name: 'cache-staging' })).toHaveAttribute('aria-selected', 'true')

    await page.keyboard.press('ControlOrMeta+k')
    await page.getByPlaceholder('Search hosts, tabs and actions').fill('new database')
    await page.keyboard.press('Enter')
    await expect(sheet(page).getByRole('heading', { name: 'New database' })).toBeVisible()
  })
})
