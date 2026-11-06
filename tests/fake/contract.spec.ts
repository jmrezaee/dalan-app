import { readFileSync } from 'node:fs'
import { expect, test } from '../support/fixtures'
import { installFakeBackend } from './index'

// The contract: every command the generated bindings can call has a fake. `pnpm typecheck` already fails when a
// handler is missing (the table is typed from the bindings); this test also checks the running page.
const bindings = readFileSync('app/types/bindings.ts', 'utf8')
const commandNames = [...bindings.matchAll(/__TAURI_INVOKE(?:<[^>]*>)?\("([a-z_]+)"/g)].map(m => m[1]!).sort()

test('the fake answers to exactly the commands in the bindings', async ({ page }) => {
  const fake = await installFakeBackend(page, { scenario: 'team', unlocked: true })
  await page.goto('/')
  expect(commandNames.length).toBeGreaterThan(90)
  expect((await fake.commandNames()).sort()).toEqual(commandNames)
})

test('every list command answers, with no secret in the answer', async ({ page }) => {
  const fake = await installFakeBackend(page, { scenario: 'team', unlocked: true })
  await page.goto('/')
  // The commands that take no arguments but a limit: the lists the screens load.
  const lists = commandNames.filter(n => /^(hosts_tree|keys_list|tags_list|workspaces_list|identities_list|known_hosts_list|logs_list|forwards_list|snippets_list|edit_apps|serial_ports|terminal_themes_list|ai_get|app_info|vault_status|settings_get_all|recent_connections|history_list)$/.test(n))
  expect(lists.length).toBeGreaterThan(12)
  for (const name of lists) {
    const answer = await page.evaluate(([cmd]) => (window as any).__TAURI_INTERNALS__.invoke(cmd, { limit: 10, hostId: null, query: null }).catch((e: any) => ({ failed: cmd, error: e })), [name!])
    expect(answer, name).not.toHaveProperty('failed')
    expect(JSON.stringify(answer) ?? 'null', name).not.toMatch(/"(password|passphrase|private_key|secret|api_key|token)":\s*"/)
  }
  expect(await fake.unhandled()).toEqual([])
})

test('an unknown command is refused loudly and recorded', async ({ page }) => {
  const fake = await installFakeBackend(page, { scenario: 'empty', unlocked: true })
  await page.goto('/')
  const error = await page.evaluate(() => (window as any).__TAURI_INTERNALS__.invoke('not_a_command').catch((e: any) => e))
  expect(error).toMatchObject({ code: 'internal' })
  expect(await fake.unhandled()).toEqual(['not_a_command'])
})
