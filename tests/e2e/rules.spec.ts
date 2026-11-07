import { expect, test, TEST_SECRETS } from '../support/fixtures'
import { MASTER } from '../fake/commands/vault'
import { fakeEvent, openApp, seen, sessionIds, unlock } from './harness'

// CLAUDE.md hard rules in the browser (replica/testing/test-plan.md section 9.1). The Vitest and Rust halves are in
// tests/rules.test.ts and src-tauri/src/*/tests.rs.

test.describe('rule 1: secrets never reach JavaScript', () => {
  test('passwords, passphrases and keys go in, and only flags come out', async ({ page }) => {
    const fake = await openApp(page, { scenario: 'team', unlocked: true })
    // Send every kind of secret in through the real command path, then read the vault back.
    await page.evaluate(async () => {
      const invoke = (window as any).__TAURI_INTERNALS__.invoke
      await invoke('host_save', { input: { id: null, label: 'secret-host', address: '10.9.9.9', port: null, username: 'me', password: 'hunter2-test-secret', clear_password: false, group_id: null, key_id: null, identity_id: null, startup_snippet_id: null, jump_mode: 'inherit', jump_host_ids: [], tags: [] } })
      await invoke('key_generate', { input: { label: 'pp key', kind: 'ed25519', passphrase: 'pp-test-secret', save_passphrase: true } })
      await invoke('identity_save', { input: { id: null, label: 'id', username: 'me', password: 'a-test-password', clear_password: false, key_id: null } })
      await invoke('ai_save', { input: { model: 'm', base_url: 'https://api.example.com/v1', api_key: 'sk-ant-test-key', clear_key: false } })
      await invoke('hosts_tree'); await invoke('keys_list'); await invoke('identities_list'); await invoke('ai_get')
    })
    const tree = JSON.parse((await fake.answers()).filter(a => a.startsWith('hosts_tree ')).pop()!.slice('hosts_tree '.length))
    expect(tree.hosts.find((h: any) => h.label === 'secret-host')).toMatchObject({ has_password: true })
    const all = (await fake.answers()).join('\n')
    for (const secret of TEST_SECRETS) expect(all, secret).not.toContain(secret)
    expect(all).toContain('"has_key":true')
  })

  test('the master password is not echoed back either', async ({ page }) => {
    const fake = await openApp(page)
    await unlock(page, MASTER)
    await seen(page.getByLabel('Vault', { exact: true }))
    expect((await fake.answers()).join('\n')).not.toContain(MASTER)
  })
})

test('rule 3: terminal data is bytes, so a character split between two chunks still draws', async ({ page }) => {
  const fake = await openApp(page, { scenario: 'team', unlocked: true })
  await page.getByText('roaming laptop', { exact: true }).first().dblclick()
  const [id] = await sessionIds(page)
  await fakeEvent(page, id!, { type: 'state', state: 'ready' })
  // 🙂 is F0 9F 99 82: cut it in the middle, and add bytes that are not text at all.
  await fake.session(id!).outputBytes([0x41, 0xF0, 0x9F])
  await fake.session(id!).outputBytes([0x99, 0x82, 0x42, 0x0D, 0x0A])
  await expect.poll(() => page.evaluate(() => {
    const term = Object.values((window as any).__dalanTerm)[0] as any
    return term.buffer.active.getLine(0).translateToString(true)
  })).toBe('A🙂B')
})

test('rule 4: a live terminal stays mounted through tab switches and Settings', async ({ page }) => {
  const fake = await openApp(page, { scenario: 'team', unlocked: true })
  await page.getByText('roaming laptop', { exact: true }).first().dblclick()
  const [id] = await sessionIds(page)
  await fakeEvent(page, id!, { type: 'state', state: 'ready' })
  await fake.session(id!).output('marker-in-tab-1\r\n')
  await expect.poll(() => page.evaluate(() => Object.values((window as any).__dalanTerm ?? {}).length)).toBe(1)
  await page.evaluate(() => { (window as any).__first = Object.values((window as any).__dalanTerm)[0] })
  for (let i = 0; i < 10; i++) {
    await page.getByRole('tab', { name: i % 2 ? /roaming laptop/ : 'SFTP' }).click()
  }
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur()) // off a Mac, Ctrl keys typed in a terminal go to the shell
  await page.keyboard.press('ControlOrMeta+,')
  await seen(page.getByRole('button', { name: 'Appearance', exact: true }).or(page.getByRole('tab', { name: 'Appearance', exact: true })))
  await page.getByRole('tab', { name: /roaming laptop/ }).click()
  expect(await page.evaluate(() => (window as any).__first === Object.values((window as any).__dalanTerm)[0])).toBe(true) // the same xterm instance
  expect(await page.evaluate(() => (window as any).__first.buffer.active.getLine(0).translateToString(true))).toContain('marker-in-tab-1')
})

test('rule 7: an error code the app has never seen still shows friendly text, not the raw code', async ({ page }) => {
  const fake = await openApp(page, { scenario: 'team', unlocked: true })
  await fake.failNext('host_save', { code: 'brand_new_code', message: 'RAW BACKEND TEXT' })
  await page.getByRole('button', { name: 'New host', exact: true }).click()
  await page.getByLabel('Address').fill('10.1.1.1')
  await page.getByRole('button', { name: /^(Save|Connect)/ }).first().click()
  await seen(page.getByText('Something went wrong. Try again.'))
  await expect(page.getByText('brand_new_code')).toHaveCount(0)
  await expect(page.getByText('RAW BACKEND TEXT')).toHaveCount(0)
})
