import { expect, test } from '../support/fixtures'
import type { Page } from '@playwright/test'
import { openApp } from './harness'

// R3 of Redis (replica/redis/spec.md section 4): the write gate and the editors (S28, S29, S33; F15).
// The team scenario has one database per mode: `local` is Open, `cache-prod` is Confirm writes (production),
// `sessions-prod` is Read-only (production). The fake backend's gate answers like Rust's.
async function openDb(page: Page, name: string) {
  const fake = await openApp(page, { scenario: 'team', unlocked: true })
  await page.getByRole('navigation', { name: 'Vault' }).getByRole('link', { name: /Databases/ }).click()
  await page.getByRole('row', { name: new RegExp(name) }).dblclick()
  const tab = page.getByTestId(`redis-tab-${name}`)
  await expect(tab.getByRole('tree', { name: 'Keys' })).toBeVisible()
  return { fake, tab, value: tab.getByTestId('redis-value') }
}
const item = (tab: ReturnType<Page['getByTestId']>, name: string | RegExp) => tab.getByRole('treeitem', { name })
async function openKey(tab: ReturnType<Page['getByTestId']>, folder: RegExp | null, key: RegExp) {
  if (folder) {
    await item(tab, /^user:/).click()
    await item(tab, folder).click()
  }
  await item(tab, key).first().click()
}
const confirm = (page: Page) => page.getByTestId('redis-confirm')
const danger = (page: Page) => page.getByTestId('redis-danger')

test.describe('Read-only (S33)', () => {
  test('the banner says so, every add, edit and delete control is hidden, and the mode button says Read-only', async ({ page }) => {
    const { tab, value } = await openDb(page, 'sessions-prod')
    await expect(tab.getByTestId('redis-read-only')).toContainText('Read-only. Nothing here can change data.')
    await expect(tab.getByTestId('redis-mode')).toHaveText('Read-only')
    await expect(tab.getByRole('button', { name: 'New key' })).toHaveCount(0)

    await openKey(tab, /^1042:/, /profile/)
    await expect(value.getByRole('table', { name: 'Fields' })).toBeVisible()
    for (const name of ['Add field', 'Rename', 'Delete key', /^Edit field/, /^Remove field/]) await expect(value.getByRole('button', { name })).toHaveCount(0)
    // The expiry is text, not a control.
    await expect(value.getByText('Expires in 23h 14m')).toBeVisible()
    await expect(value.getByRole('button', { name: /Expires in/ })).toHaveCount(0)
    // Enter and Delete on a row do nothing.
    await value.getByRole('table', { name: 'Fields' }).focus()
    await page.keyboard.press('Enter')
    await page.keyboard.press('Delete')
    await expect(page.getByTestId('redis-edit-row')).toHaveCount(0)
    await expect(confirm(page)).toHaveCount(0)

    // A string is shown, not editable.
    await item(tab, /maintenance/).click()
    await expect(value.locator('pre[aria-label="Value as text"]')).toContainText('Back at 14:30 UTC.')
    await expect(value.locator('textarea')).toHaveCount(0)
  })

  test('the gate refuses a write even when the page is made to ask for one', async ({ page }) => {
    const { fake, tab } = await openDb(page, 'sessions-prod')
    await expect(tab.getByTestId('redis-read-only')).toBeVisible()
    const conn = (await fake.redisOpenConnIds()).at(-1)!
    // Straight to the backend, past every hidden button: the answer is still no.
    const answer = await page.evaluate(async (connId) => {
      const invoke = (window as any).__TAURI_INTERNALS__.invoke
      try {
        return await invoke('redis_write_prepare', { connId, op: { op: 'delete', key: { kind: 'text', text: 'maintenance' } } })
      }
      catch (e) {
        return e
      }
    }, conn)
    expect(answer).toMatchObject({ code: 'redis_read_only_mode' })
    expect(await fake.redisKey('maintenance')).not.toBeNull()
    expect(await fake.calls()).toContain('redis_refused:delete')
  })
})

test.describe('Confirm writes (S33)', () => {
  test('editing a hash field shows the database, the target, now, after and the exact command; Cancel is focused and sends nothing', async ({ page }) => {
    const { fake, tab, value } = await openDb(page, 'cache-prod')
    await expect(tab.getByTestId('redis-read-only')).toHaveCount(0)
    await openKey(tab, /^1042:/, /profile/)
    await value.getByRole('button', { name: 'Edit field plan' }).click()
    const row = page.getByTestId('redis-edit-row')
    await expect(row.getByLabel('Value')).toHaveValue('team')
    await expect(row.getByLabel('Value')).toBeFocused()
    await row.getByLabel('Value').fill('business')
    await row.getByRole('button', { name: 'Review change' }).click()

    const d = confirm(page)
    await expect(d.getByRole('heading', { name: 'Change this field?' })).toBeVisible()
    await expect(d).toContainText('cache-prod · production')
    await expect(d).toContainText('user:1042:profile › plan')
    await expect(d.locator('[data-part="now"]')).toHaveText('team')
    await expect(d.locator('[data-part="after"]')).toHaveText('business')
    await expect(d.locator('[data-part="command"]')).toHaveText('HSET user:1042:profile plan business')
    await expect(d.getByRole('button', { name: 'Cancel' })).toBeFocused()

    // Enter on the focused Cancel cancels: nothing was sent, and the row is still being edited.
    await page.keyboard.press('Enter')
    await expect(d).toHaveCount(0)
    expect((await fake.redisKey('user:1042:profile'))!.value.fields).toContainEqual(['plan', 'team'])
    await expect(row.getByLabel('Value')).toHaveValue('business')

    // Review again and confirm: the field changes, the row closes, the table shows the new value.
    await row.getByRole('button', { name: 'Review change' }).click()
    await confirm(page).getByRole('button', { name: 'Change', exact: true }).click()
    await expect(confirm(page)).toHaveCount(0)
    await expect(page.getByTestId('redis-edit-row')).toHaveCount(0)
    await expect(value.getByRole('row', { name: /plan/ })).toContainText('business')
    expect((await fake.redisKey('user:1042:profile'))!.value.fields).toContainEqual(['plan', 'business'])
    expect((await fake.calls()).filter(c => c === 'redis_write:hash_set')).toHaveLength(1)
  })

  test('keyboard: Enter edits the focused row, Esc cancels, Enter in the edit asks for review; Delete asks to remove', async ({ page }) => {
    const { fake, tab, value } = await openDb(page, 'cache-prod')
    await openKey(tab, /^1042:/, /profile/)
    const table = value.getByRole('table', { name: 'Fields' })
    await table.focus()
    await page.keyboard.press('ArrowDown') // email
    await page.keyboard.press('Enter')
    const row = page.getByTestId('redis-edit-row')
    await expect(row.getByLabel('Value')).toHaveValue('sara@example.test')
    await page.keyboard.press('Escape')
    await expect(row).toHaveCount(0)

    await table.focus()
    await page.keyboard.press('Enter')
    await row.getByLabel('Value').fill('sara@example.org')
    await page.keyboard.press('Enter')
    await expect(confirm(page).locator('[data-part="command"]')).toHaveText('HSET user:1042:profile email sara@example.org')
    await page.keyboard.press('Escape')
    await expect(confirm(page)).toHaveCount(0)
    await page.keyboard.press('Escape')

    await table.focus()
    await page.keyboard.press('Delete')
    const d = confirm(page)
    await expect(d.getByRole('heading', { name: 'Remove this field?' })).toBeVisible()
    await expect(d.locator('[data-part="now"]')).toHaveText('sara@example.test')
    await expect(d.locator('[data-part="command"]')).toHaveText('HDEL user:1042:profile email')
    await d.getByRole('button', { name: 'Remove field' }).click()
    await expect(value.getByRole('row', { name: /email/ })).toHaveCount(0)
    expect((await fake.redisKey('user:1042:profile'))!.value.fields.map((f: string[]) => f[0])).not.toContain('email')
  })

  test('a string is edited as text and reviewed before it is saved; the expiry and a rename go through the same dialog', async ({ page }) => {
    const { fake, tab, value } = await openDb(page, 'cache-prod')
    await item(tab, /maintenance/).click()
    const text = value.getByRole('textbox', { name: 'Value as text' })
    await expect(text).toHaveValue('Back at 14:30 UTC. Deploys are paused.')
    await expect(value.getByRole('button', { name: 'Review change' })).toBeDisabled()
    await text.fill('Back at 15:00 UTC.')
    await value.getByRole('button', { name: 'Review change' }).click()
    await expect(confirm(page).locator('[data-part="now"]')).toHaveText('Back at 14:30 UTC. Deploys are paused.')
    await expect(confirm(page).locator('[data-part="command"]')).toHaveText('SET maintenance "Back at 15:00 UTC." KEEPTTL')
    await confirm(page).getByRole('button', { name: 'Change', exact: true }).click()
    await expect(text).toHaveValue('Back at 15:00 UTC.')
    await expect(value.getByRole('button', { name: 'Review change' })).toBeDisabled()

    // Expiry: words a person would type. A wrong one is explained, not sent.
    await value.getByRole('button', { name: /Expires in 12m/ }).click()
    const expiry = value.getByLabel('Expires in')
    await expiry.fill('soon')
    await expiry.press('Enter')
    await expect(value.getByRole('alert')).toHaveText('Use 90s, 15m, 2h, 7d or a date in the future.')
    await expiry.fill('2h')
    await expiry.press('Enter')
    await expect(confirm(page).locator('[data-part="command"]')).toHaveText('PEXPIRE maintenance 7200000')
    await confirm(page).getByRole('button', { name: 'Set expiry' }).click()
    await expect(value.getByRole('button', { name: /Expires in 2h/ })).toBeVisible()
    await expect(item(tab, /maintenance/)).toContainText('2h')
    // Remove expiry.
    await value.getByRole('button', { name: /Expires in 2h/ }).click()
    await value.getByRole('button', { name: 'Remove expiry' }).click()
    await expect(confirm(page).locator('[data-part="command"]')).toHaveText('PERSIST maintenance')
    await confirm(page).getByRole('button', { name: 'Remove expiry' }).click()
    await expect(value.getByRole('button', { name: 'No expiry' })).toBeVisible()

    // Rename never overwrites.
    await value.getByRole('button', { name: 'Rename' }).click()
    await value.getByLabel('New name').fill('flags:beta')
    await value.getByLabel('New name').press('Enter')
    await expect(confirm(page).locator('[data-part="command"]')).toHaveText('RENAMENX maintenance flags:beta')
    await confirm(page).getByRole('button', { name: 'Rename' }).click()
    await expect(page.getByText('A key with that name already exists. Nothing was changed.')).toBeVisible()
    expect(await fake.redisKey('maintenance')).not.toBeNull()
    // The form is still open, so the name can be fixed.
    await value.getByLabel('New name').fill('notice:maintenance')
    await value.getByLabel('New name').press('Enter')
    await confirm(page).getByRole('button', { name: 'Rename' }).click()
    await expect(value.getByRole('heading', { name: 'notice:maintenance' })).toBeVisible()
    expect(await fake.redisKey('maintenance')).toBeNull()
    expect((await fake.redisKey('notice:maintenance'))!.value.value.text).toBe('Back at 15:00 UTC.')
  })

  test('deleting a key asks first; a new key is reviewed and never overwrites', async ({ page }) => {
    const { fake, tab, value } = await openDb(page, 'cache-prod')
    await item(tab, /maintenance/).click()
    await value.getByRole('button', { name: 'Delete key' }).click()
    const d = confirm(page)
    await expect(d.getByRole('heading', { name: 'Delete maintenance?' })).toBeVisible()
    await expect(d.locator('[data-part="command"]')).toHaveText('UNLINK maintenance')
    await expect(d.getByRole('button', { name: 'Cancel' })).toBeFocused()
    await d.getByRole('button', { name: 'Delete key' }).click()
    await expect(item(tab, /maintenance/)).toHaveCount(0)
    await expect(page.getByRole('heading', { name: 'Pick a key to see its value.' })).toBeVisible()
    expect(await fake.redisKey('maintenance')).toBeNull()

    await tab.getByRole('button', { name: 'New key' }).click()
    const form = page.getByTestId('redis-new-key')
    await form.getByLabel('Name').fill('flags:beta')
    await form.getByLabel('Value').fill('on')
    await form.getByRole('button', { name: 'Review change' }).click()
    await confirm(page).getByRole('button', { name: 'Create key' }).click()
    await expect(page.getByText('A key with that name already exists. Nothing was changed.')).toBeVisible()

    await tab.getByRole('button', { name: 'New key' }).click()
    await form.getByLabel('Name').fill('notice:banner')
    await form.getByLabel('Value').fill('Hello')
    await form.getByLabel('Expiry (optional)').fill('15m')
    await form.getByRole('button', { name: 'Review change' }).click()
    await expect(confirm(page).getByRole('heading', { name: 'Create this key?' })).toBeVisible()
    await confirm(page).getByRole('button', { name: 'Create key' }).click()
    await expect(value.getByRole('heading', { name: 'notice:banner' })).toBeVisible()
    await expect(value.getByRole('textbox', { name: 'Value as text' })).toHaveValue('Hello')
    expect((await fake.redisKey('notice:banner'))!.ttl_ms).toBe(900_000)
  })

  test('a confirmation works once: asked a second time, or from another tab, the gate says no', async ({ page }) => {
    const { fake, tab, value } = await openDb(page, 'cache-prod')
    await item(tab, /maintenance/).click()
    await value.getByRole('button', { name: 'Delete key' }).click()
    await expect(confirm(page)).toBeVisible()
    const conn = (await fake.redisOpenConnIds()).at(-1)!
    const tries = await page.evaluate(async (connId) => {
      const invoke = (window as any).__TAURI_INTERNALS__.invoke
      const attempt = (args: unknown) => invoke('redis_write_commit', args).then(() => 'ran', (e: { code: string }) => e.code)
      const other = await attempt({ connId: 'another-tab', pendingId: 'rpend-1', typedName: null })
      // The pending id is the fake's first: find it by asking with the right connection twice.
      const out = await invoke('redis_write_prepare', { connId, op: { op: 'expire', key: { kind: 'text', text: 'flags:beta' }, ttl_ms: 60000 } })
      const first = await attempt({ connId, pendingId: out.pending.id, typedName: null })
      const second = await attempt({ connId, pendingId: out.pending.id, typedName: null })
      return { other, first, second }
    }, conn)
    expect(tries.other).not.toBe('ran')
    expect(tries).toMatchObject({ first: 'ran', second: 'redis_confirm_needed' })
  })
})

test.describe('dangerous: type the name (S33)', () => {
  test('deleting a key with more than 10,000 items stays disabled until the database\'s exact name is typed', async ({ page }) => {
    const { fake, tab, value } = await openDb(page, 'cache-prod')
    await fake.redisAddSet('audit:seen', 10_001)
    await tab.getByRole('button', { name: 'Refresh keys' }).click()
    await item(tab, /audit:seen/).click()
    await value.getByRole('button', { name: 'Delete key' }).click()

    const d = danger(page)
    await expect(d.getByRole('heading', { name: 'Delete audit:seen on cache-prod?' })).toBeVisible()
    await expect(d).toContainText('its 10,001 items')
    await expect(d.locator('[data-part="command"]')).toHaveText('UNLINK audit:seen')
    await expect(d.getByRole('button', { name: 'Cancel' })).toBeFocused()
    const go = d.getByRole('button', { name: 'Delete key' })
    await expect(go).toBeDisabled()
    const name = d.getByLabel('Type cache-prod to confirm')
    for (const wrong of ['cache-pro', 'Cache-prod', 'cache-prod ']) {
      await name.fill(wrong)
      await expect(go).toBeDisabled()
    }
    // Enter in the field does nothing while the name is wrong.
    await name.press('Enter')
    await expect(d).toBeVisible()
    expect(await fake.redisKey('audit:seen')).not.toBeNull()

    await name.fill('cache-prod')
    await expect(go).toBeEnabled()
    await go.click()
    await expect(d).toHaveCount(0)
    await expect(item(tab, /audit:seen/)).toHaveCount(0)
    expect(await fake.redisKey('audit:seen')).toBeNull()
  })

  test('an Open database still asks for the name, and the gate checks the name itself', async ({ page }) => {
    const { fake, tab, value } = await openDb(page, 'local')
    await fake.redisAddSet('audit:seen', 10_001)
    await tab.getByRole('button', { name: 'Refresh keys' }).click()
    await item(tab, /audit:seen/).click()
    await value.getByRole('button', { name: 'Delete key' }).click()
    await page.getByTestId('redis-delete-open').getByRole('button', { name: 'Delete key' }).click()
    await expect(danger(page).getByRole('heading', { name: 'Delete audit:seen on local?' })).toBeVisible()
    await page.keyboard.press('Escape')

    const conn = (await fake.redisOpenConnIds()).at(-1)!
    const code = await page.evaluate(async (connId) => {
      const invoke = (window as any).__TAURI_INTERNALS__.invoke
      const out = await invoke('redis_write_prepare', { connId, op: { op: 'delete', key: { kind: 'text', text: 'audit:seen' } } })
      return invoke('redis_write_commit', { connId, pendingId: out.pending.id, typedName: 'loca' }).then(() => 'ran', (e: { code: string }) => e.code)
    }, conn)
    expect(code).toBe('redis_confirm_needed')
    expect(await fake.redisKey('audit:seen')).not.toBeNull()
  })
})

test.describe('Open mode and every editor (S28, S29)', () => {
  test('edits go straight through and the buttons say Save; deleting a key still asks once', async ({ page }) => {
    const { fake, tab, value } = await openDb(page, 'local')
    await expect(tab.getByTestId('redis-mode')).toHaveText('Open')
    await openKey(tab, /^1042:/, /profile/)
    await value.getByRole('button', { name: 'Add field' }).click()
    const row = page.getByTestId('redis-edit-row')
    await row.getByLabel('Field').fill('team')
    await row.getByLabel('Value').fill('platform')
    await row.getByRole('button', { name: 'Save' }).click()
    await expect(confirm(page)).toHaveCount(0)
    await expect(value.getByRole('row', { name: /platform/ })).toBeVisible()
    await expect(value.getByText('9 fields · 412 B')).toBeVisible()
    expect((await fake.redisKey('user:1042:profile'))!.value.fields).toContainEqual(['team', 'platform'])

    await item(tab, /flags:beta/).click()
    await value.getByRole('button', { name: 'Delete key' }).click()
    const ask = page.getByTestId('redis-delete-open')
    await expect(ask.getByRole('heading', { name: 'Delete flags:beta?' })).toBeVisible()
    await expect(ask.getByRole('button', { name: 'Cancel' })).toBeFocused()
    await ask.getByRole('button', { name: 'Delete key' }).click()
    await expect(item(tab, /flags:beta/)).toHaveCount(0)
  })

  test('list: add at either end, change an item, remove by position', async ({ page }) => {
    const { fake, tab, value } = await openDb(page, 'local')
    await openKey(tab, /^1042:/, /cart/)
    const row = page.getByTestId('redis-edit-row')
    const items = async () => (await fake.redisKey('user:1042:cart'))!.value.items
    await value.getByRole('button', { name: 'Add at the start' }).click()
    await row.getByLabel('Value').fill('sku-0000')
    await row.getByLabel('Value').press('Enter')
    await expect.poll(items).toEqual(['sku-0000', 'sku-8841', 'sku-1207', 'sku-5530'])
    await value.getByRole('button', { name: 'Add at the end' }).click()
    await row.getByLabel('Value').fill('sku-9999')
    await row.getByLabel('Value').press('Enter')
    await expect.poll(items).toEqual(['sku-0000', 'sku-8841', 'sku-1207', 'sku-5530', 'sku-9999'])
    await value.getByRole('button', { name: 'Edit item 1' }).click()
    await row.getByLabel('Value').fill('sku-8842')
    await row.getByLabel('Value').press('Enter')
    await expect.poll(items).toEqual(['sku-0000', 'sku-8842', 'sku-1207', 'sku-5530', 'sku-9999'])
    await value.getByRole('button', { name: 'Remove item 0' }).click()
    await expect.poll(items).toEqual(['sku-8842', 'sku-1207', 'sku-5530', 'sku-9999'])
    await expect(value.getByText('All 4 items loaded')).toBeVisible()
  })

  test('set, sorted set and stream: add and remove; a score is changed in place; the last item takes the key with it', async ({ page }) => {
    const { fake, tab, value } = await openDb(page, 'local')
    const row = page.getByTestId('redis-edit-row')
    await openKey(tab, /^1042:/, /roles/)
    await value.getByRole('button', { name: 'Add member' }).click()
    await row.getByLabel('Member').fill('admin')
    await row.getByLabel('Member').press('Enter')
    await expect(value.getByRole('row', { name: /admin/ })).toBeVisible()
    await value.getByRole('button', { name: 'Remove member billing' }).click()
    await expect.poll(async () => (await fake.redisKey('user:1042:roles'))!.value.members).toEqual(['editor', 'owner', 'admin'])
    await expect(value.getByRole('button', { name: /^Edit member/ })).toHaveCount(0)

    await item(tab, /leaderboard:weekly/).click()
    await value.getByRole('button', { name: 'Edit member 9812' }).click()
    await row.getByLabel('Score').fill('high')
    await row.getByLabel('Score').press('Enter')
    await expect(page.getByText('A score is a number, inf or -inf.')).toBeVisible()
    await row.getByLabel('Score').fill('12000')
    await row.getByLabel('Score').press('Enter')
    await expect(value.getByRole('row', { name: /team:0000/ })).toContainText('12000')
    await value.getByRole('button', { name: 'Add member' }).click()
    await row.getByLabel('Score').fill('99999')
    await row.getByLabel('Member').fill('team:new')
    await row.getByLabel('Member').press('Enter')
    await expect(value.getByRole('row', { name: /team:new/ })).toContainText('99999')

    await item(tab, /events:orders/).click()
    await value.getByRole('button', { name: 'Add entry' }).click()
    await row.getByLabel('Fields').fill('order 5513')
    await row.getByLabel('Fields').press('Enter')
    await expect(page.getByText('Write the fields as name=value, with a space between them.')).toBeVisible()
    await row.getByLabel('Fields').fill('order=5513 status=paid')
    await row.getByLabel('Fields').press('Enter')
    await expect(value.getByRole('row', { name: /order=5513 status=paid/ })).toBeVisible()
    await value.getByRole('button', { name: 'Remove entry 1794770031-1' }).click()
    await expect(value.getByRole('row', { name: /1794770031-1/ })).toHaveCount(0)

    // A set that loses its only member is gone, and the browser says so.
    await item(tab, /^1044:/).click()
    await item(tab, /roles/).last().click()
    await value.getByRole('button', { name: 'Remove member viewer' }).click()
    await expect(page.getByText('That was its last item, so the key is gone.')).toBeVisible()
    await expect(page.getByRole('heading', { name: 'Pick a key to see its value.' })).toBeVisible()
    expect(await fake.redisKey('user:1044:roles')).toBeNull()
  })

  test('a JSON key is edited as text and must stay valid; binary and preview values stay read-only', async ({ page }) => {
    const { fake, tab, value } = await openDb(page, 'local')
    await item(tab, /config:pricing/).click()
    await value.getByRole('radio', { name: 'Text' }).click()
    const text = value.getByRole('textbox', { name: 'Value as text' })
    await text.fill('{"currency":"CAD",')
    await expect(value.getByRole('alert')).toContainText('isn\'t valid JSON yet.')
    await expect(value.getByRole('button', { name: 'Save' })).toBeDisabled()
    await text.fill('{"currency":"EUR"}')
    await value.getByRole('button', { name: 'Save' }).click()
    await expect.poll(async () => (await fake.redisKey('config:pricing'))!.value.text).toBe('{"currency":"EUR"}')

    await item(tab, /blob:thumb/).click()
    await expect(value.getByText('Binary values open as hex and are read-only.')).toBeVisible()
    await expect(value.locator('textarea')).toHaveCount(0)
    await item(tab, /dump:big/).click()
    await expect(value.getByText(/Showing the first/)).toBeVisible()
    await expect(value.locator('textarea')).toHaveCount(0)
  })
})

test.describe('changing the mode (S33)', () => {
  test('loosening a production database asks for the master password, and a wrong one changes nothing', async ({ page }) => {
    const { fake, tab } = await openDb(page, 'sessions-prod')
    await tab.getByTestId('redis-read-only').getByRole('button', { name: 'Change…' }).click()
    const d = page.getByTestId('redis-mode-dialog')
    await expect(d.getByRole('heading', { name: 'Writes on sessions-prod' })).toBeVisible()
    await expect(d.getByRole('radio', { name: /Read-only/ })).toHaveAttribute('aria-checked', 'true')
    await expect(d.getByRole('button', { name: 'Change mode' })).toBeDisabled()
    await expect(d.getByLabel('Master password')).toHaveCount(0)

    await d.getByRole('radio', { name: /Confirm writes/ }).click()
    await expect(d.getByText('Making a production database less protected needs your master password.')).toBeVisible()
    await d.getByRole('button', { name: 'Change mode' }).click()
    await expect(d.getByRole('alert')).toHaveText('Type your master password.')
    await d.getByLabel('Master password').fill('not-the-password')
    await d.getByRole('button', { name: 'Change mode' }).click()
    await expect(d.getByRole('alert')).toHaveText('That password didn\'t work.')
    expect(await fake.redisMode('sessions-prod')).toBe('read_only')
    await expect(tab.getByTestId('redis-read-only')).toBeVisible()

    await d.getByLabel('Master password').fill('correct-horse')
    await d.getByRole('button', { name: 'Change mode' }).click()
    await expect(d).toHaveCount(0)
    expect(await fake.redisMode('sessions-prod')).toBe('confirm')
    // The open tab follows at once: the banner goes and the editors appear.
    await expect(tab.getByTestId('redis-read-only')).toHaveCount(0)
    await expect(tab.getByTestId('redis-mode')).toHaveText('Confirm writes')
    await expect(tab.getByRole('button', { name: 'New key' })).toBeVisible()
  })

  test('tightening needs no password; the editor cannot loosen a production database', async ({ page }) => {
    const { fake, tab } = await openDb(page, 'cache-prod')
    await tab.getByTestId('redis-mode').click()
    const d = page.getByTestId('redis-mode-dialog')
    await d.getByRole('radio', { name: /Read-only/ }).click()
    await expect(d.getByLabel('Master password')).toHaveCount(0)
    await d.getByRole('button', { name: 'Change mode' }).click()
    await expect(tab.getByTestId('redis-read-only')).toBeVisible()
    expect(await fake.redisMode('cache-prod')).toBe('read_only')

    await page.getByRole('tab', { name: 'Vault' }).click()
    await page.getByRole('row', { name: /cache-prod/ }).click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Edit' }).click()
    const sheet = page.getByRole('dialog', { name: /cache-prod|database/i })
    await expect(sheet.getByRole('radio', { name: /Confirm writes/ })).toBeDisabled()
    await expect(sheet.getByRole('radio', { name: /^Open/ })).toBeDisabled()
    await expect(sheet.getByText('Loosen this from the mode button in its tab (it asks for your master password).')).toBeVisible()
  })
})
