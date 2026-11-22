// Every screen state the accessibility check opens. Each entry gets the page into the state; the spec runs axe on it.
// IDs follow replica/testing/test-plan.md section 7 (screen, state).
import type { Page } from '@playwright/test'
import type { Fake, FakeOptions  } from '../fake'

export interface ScreenState {
  id: string
  scenario: FakeOptions['scenario']
  /** Start unlocked (default) or on the unlock screen. */
  unlocked?: boolean
  open: (page: Page, fake: Fake) => Promise<void>
}

const vaultNav = (page: Page, name: string) => page.getByLabel('Vault', { exact: true }).getByText(name, { exact: true }).click()
const hostRow = (page: Page, name: string) => page.getByRole('row').filter({ has: page.getByText(name, { exact: true }) }).first()
const open = async (page: Page, fake: Fake, name: string) => {
  await hostRow(page, name).dblclick()
  const ids = await fake.sessionIds()
  return ids[ids.length - 1]!
}

/** Adds a database of another engine through the picker and the form (the fake keeps it). */
const addDb = async (page: Page, engine: string, name: string, address: string) => {
  await vaultNav(page, 'Databases')
  await page.getByRole('button', { name: 'New database' }).click()
  await page.getByRole('radio', { name: new RegExp(engine) }).click()
  await page.getByRole('button', { name: 'Continue' }).click()
  const sheet = page.getByTestId('db-form-sheet')
  await sheet.getByLabel('Name', { exact: true }).fill(name)
  await sheet.getByLabel(engine === 'MongoDB' ? 'Members' : 'Address').fill(address)
  await sheet.getByRole('button', { name: 'Save' }).click()
  await sheet.waitFor({ state: 'detached' })
}

/** Adds a PostgreSQL database and opens it in a tab (the fake plays the connection). */
const sqlTab = async (page: Page) => {
  await addDb(page, 'PostgreSQL', 'app-prod', 'db.internal')
  await page.getByRole('row', { name: /app-prod/ }).dblclick()
  const tab = page.getByTestId('sql-tab-app-prod')
  await tab.getByRole('tree', { name: 'Schemas and tables' }).waitFor()
  return tab
}

/** Opens a saved Redis database in a tab (the fake connects by itself) and, if asked, one of its views. */
const redisTab = async (page: Page, name: string, view?: 'Console' | 'Pub/Sub' | 'Server') => {
  await vaultNav(page, 'Databases')
  await page.getByRole('row', { name: new RegExp(name) }).dblclick()
  const tab = page.getByTestId(`redis-tab-${name}`)
  await tab.getByRole('tree', { name: 'Keys' }).waitFor()
  if (view) await tab.getByRole('tab', { name: view }).click()
  return tab
}
/** Opens user:1042:<key> in the tree. */
const redisKey = async (tab: ReturnType<Page['getByTestId']>, key: string) => {
  await tab.getByRole('treeitem', { name: /^user:/ }).click()
  await tab.getByRole('treeitem', { name: /^1042:/ }).click()
  await tab.getByRole('treeitem', { name: new RegExp(key) }).first().click()
  await tab.getByTestId('redis-value').getByRole('table').waitFor()
}

export const SCREENS: ScreenState[] = [
  { id: 'S24-first-run', scenario: 'fresh', unlocked: false, open: async () => {} },
  { id: 'S24-unlock', scenario: 'team', unlocked: false, open: async () => {} },
  { id: 'S02-empty', scenario: 'empty', open: async () => {} },
  { id: 'S02-grid', scenario: 'team', open: async () => {} },
  { id: 'S02-list', scenario: 'team', open: async (p) => { await p.getByRole('button', { name: 'List view' }).click() } },
  { id: 'S02-search-no-results', scenario: 'team', open: async (p) => { await p.getByPlaceholder('Search hosts').fill('zzzz') } },
  { id: 'S02-context-menu', scenario: 'team', open: async (p) => { await hostRow(p, 'bastion').click({ button: 'right' }) } },
  { id: 'S03-new-host', scenario: 'team', open: async (p) => { await p.getByRole('button', { name: 'New host', exact: true }).click() } },
  { id: 'S03-edit-jump-page', scenario: 'team', open: async (p) => { await hostRow(p, 'bastion').click({ button: 'right' }); await p.getByRole('menuitem', { name: 'Edit', exact: true }).click(); await p.getByRole('button', { name: /^Jump hosts/ }).click() } },
  { id: 'S04-new-group', scenario: 'team', open: async (p) => { await p.getByRole('button', { name: 'More create options' }).click(); await p.getByRole('menuitem', { name: 'New group' }).click() } },
  { id: 'S05-keychain', scenario: 'team', open: async (p) => { await vaultNav(p, 'Keychain') } },
  { id: 'S06-generate', scenario: 'team', open: async (p) => { await vaultNav(p, 'Keychain'); await p.getByRole('button', { name: 'Generate key', exact: true }).click() } },
  { id: 'S06-import', scenario: 'team', open: async (p) => { await vaultNav(p, 'Keychain'); await p.getByRole('button', { name: 'More create options' }).click(); await p.getByRole('menuitem', { name: /Import key/ }).click() } },
  { id: 'S07-identity', scenario: 'team', open: async (p) => { await vaultNav(p, 'Keychain'); await p.getByRole('button', { name: 'More create options' }).click(); await p.getByRole('menuitem', { name: 'New identity' }).click() } },
  { id: 'S09-tunnels', scenario: 'team', open: async (p) => { await vaultNav(p, 'Port forwarding') } },
  { id: 'S10-wizard', scenario: 'team', open: async (p) => { await vaultNav(p, 'Port forwarding'); await p.getByRole('button', { name: /New (rule|tunnel)/ }).first().click() } },
  { id: 'S11-snippets', scenario: 'team', open: async (p) => { await vaultNav(p, 'Snippets') } },
  { id: 'S13-run-on-hosts', scenario: 'team', open: async (p) => { await vaultNav(p, 'Snippets'); await p.getByRole('row').or(p.locator('div').filter({ hasText: /^Update packages/ })).filter({ hasText: 'Update packages' }).getByRole('button', { name: 'Run' }).first().click() } },
  { id: 'S14-known-hosts', scenario: 'team', open: async (p) => { await vaultNav(p, 'Known hosts') } },
  { id: 'S15-logs', scenario: 'team', open: async (p) => { await vaultNav(p, 'Logs') } },
  { id: 'S16-new-tab', scenario: 'team', open: async (p) => { await p.keyboard.press('ControlOrMeta+t') } },
  { id: 'S17-palette', scenario: 'team', open: async (p) => { await p.keyboard.press('ControlOrMeta+k') } },
  { id: 'S18-connecting', scenario: 'team', open: async (p, f) => { const id = await open(p, f, 'bastion'); await f.session(id).event({ type: 'step', text: 'Connecting to bastion.example.com:22.' }) } },
  { id: 'S18-first-connect', scenario: 'team', open: async (p, f) => { const id = await open(p, f, 'roaming laptop'); await f.session(id).ask({ type: 'host_key_new', host: 'roaming laptop', key_type: 'ssh-ed25519', fingerprint: 'SHA256:abc' }) } },
  { id: 'S18-identity-changed', scenario: 'team', open: async (p, f) => { const id = await open(p, f, 'bastion'); await f.session(id).ask({ type: 'host_key_changed', host: 'bastion', key_type: 'ssh-ed25519', old_fingerprint: 'SHA256:old', new_fingerprint: 'SHA256:new' }) } },
  { id: 'S18-password', scenario: 'team', open: async (p, f) => { const id = await open(p, f, 'bastion'); await f.session(id).ask({ type: 'password', host: 'bastion', user: 'ops' }) } },
  { id: 'S18-passphrase', scenario: 'team', open: async (p, f) => { const id = await open(p, f, 'bastion'); await f.session(id).ask({ type: 'passphrase', key_label: 'work laptop' }) } },
  { id: 'S18-keyboard-interactive', scenario: 'team', open: async (p, f) => { const id = await open(p, f, 'bastion'); await f.session(id).ask({ type: 'keyboard_interactive', host: 'bastion', name: 'Two-step sign-in', instructions: '', prompt: 'Verification code:', echo: true }) } },
  { id: 'S18-sign-in-failed', scenario: 'team', open: async (p, f) => { const id = await open(p, f, 'bastion'); await f.session(id).event({ type: 'auth_methods', methods: ['password', 'keyboard_interactive'] }); await f.session(id).error('auth_failed', 'bastion rejected the password.') } },
  { id: 'S18-unreachable', scenario: 'team', open: async (p, f) => { const id = await open(p, f, 'bastion'); await f.session(id).error('host_unreachable', 'Can\'t reach bastion.example.com:22: timed out') } },
  { id: 'S18-disconnected', scenario: 'team', open: async (p, f) => { const id = await open(p, f, 'bastion'); await f.session(id).state('ready'); await f.session(id).state('disconnected') } },
  { id: 'S18-connected', scenario: 'team', open: async (p, f) => { const id = await open(p, f, 'bastion'); await f.session(id).connect(); await f.session(id).runScript('ubuntu-login') } },
  { id: 'S18-dropped-card', scenario: 'team', open: async (p, f) => { const a = await open(p, f, 'bastion'); await f.session(a).state('ready'); await p.getByRole('tab', { name: 'Vault' }).click(); const b = await open(p, f, 'roaming laptop'); await f.session(b).state('ready'); await f.dropAll() } },
  { id: 'S20-side-panel', scenario: 'team', open: async (p, f) => { const id = await open(p, f, 'bastion'); await f.session(id).state('ready'); await p.getByRole('button', { name: /^Snippets/ }).click() } },
  { id: 'S21-sftp', scenario: 'team', open: async (p) => { await p.getByRole('tab', { name: 'SFTP' }).click() } },
  { id: 'S23-settings-terminal', scenario: 'team', open: async (p) => { await p.keyboard.press('ControlOrMeta+,'); await p.getByRole('button', { name: 'Terminal', exact: true }).or(p.getByRole('tab', { name: 'Terminal', exact: true })).first().click() } },
  { id: 'S23-settings-shortcuts', scenario: 'team', open: async (p) => { await p.keyboard.press('ControlOrMeta+,'); await p.getByRole('button', { name: 'Shortcuts', exact: true }).or(p.getByRole('tab', { name: 'Shortcuts', exact: true })).first().click() } },
  { id: 'S23-settings-ai', scenario: 'team', open: async (p) => { await p.keyboard.press('ControlOrMeta+,'); await p.getByRole('button', { name: 'AI', exact: true }).or(p.getByRole('tab', { name: 'AI', exact: true })).first().click() } },
  // ---- SQL and MongoDB (replica/databases/spec.md): S34, S35.
  { id: 'S34-databases-all-engines', scenario: 'team', open: async (p) => { await addDb(p, 'MySQL', 'legacy-crm', '10.0.4.8'); await addDb(p, 'MongoDB', 'shop-docs', 'h1:27017,h2:27017') } },
  { id: 'S35-engine-picker', scenario: 'team', open: async (p) => { await vaultNav(p, 'Databases'); await p.getByRole('button', { name: 'New database' }).click() } },
  { id: 'S35-database-form', scenario: 'team', open: async (p) => { await vaultNav(p, 'Databases'); await p.getByRole('button', { name: 'New database' }).click(); await p.getByRole('button', { name: 'Continue' }).click(); await p.getByRole('radio', { name: 'Through a host' }).click() } },
  { id: 'S36-sql-tab-tree', scenario: 'team', open: async (p) => { await sqlTab(p) } },
  { id: 'S36-sql-table', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByRole('treeitem', { name: 'table orders' }).click(); await t.getByRole('grid', { name: 'public.orders rows' }).getByRole('gridcell', { name: '1' }).first().click() } },
  { id: 'S34-saved-queries', scenario: 'team', open: async (p) => { await vaultNav(p, 'Saved queries') } },
  // ---- Redis (replica/redis/spec.md): S26 to S33.
  { id: 'S26-databases', scenario: 'team', open: async (p) => { await vaultNav(p, 'Databases') } },
  { id: 'S27-database-sheet', scenario: 'team', open: async (p) => { await vaultNav(p, 'Databases'); await p.getByRole('button', { name: 'New database' }).click(); await p.getByRole('radio', { name: /Redis/ }).click(); await p.getByRole('button', { name: 'Continue' }).click(); await p.getByRole('radio', { name: 'Sentinel' }).click() } },
  { id: 'S28-browser-hash', scenario: 'team', open: async (p) => { const t = await redisTab(p, 'cache-prod'); await redisKey(t, 'profile') } },
  { id: 'S28-read-only', scenario: 'team', open: async (p) => { const t = await redisTab(p, 'sessions-prod'); await t.getByRole('treeitem', { name: /leaderboard:weekly/ }).click(); await t.getByRole('table', { name: 'Members' }).waitFor() } },
  { id: 'S29-edit-row', scenario: 'team', open: async (p) => { const t = await redisTab(p, 'cache-prod'); await redisKey(t, 'profile'); await t.getByRole('button', { name: 'Edit field plan' }).click() } },
  { id: 'S29-string', scenario: 'team', open: async (p) => { const t = await redisTab(p, 'cache-prod'); await t.getByRole('treeitem', { name: /maintenance/ }).click(); await t.getByRole('textbox', { name: 'Value as text' }).waitFor() } },
  { id: 'S30-console', scenario: 'team', open: async (p) => {
    const t = await redisTab(p, 'cache-prod', 'Console')
    const input = t.getByRole('textbox', { name: 'Command' })
    for (const line of ['HGETALL user:1043:profile', 'KEYS *', 'EXPIRE maintenance 600']) { await input.fill(line); await input.press('Enter') }
    await input.fill('HSE')
  } },
  { id: 'S31-server', scenario: 'team', open: async (p) => { const t = await redisTab(p, 'events-cluster', 'Server'); await t.getByRole('table', { name: 'Nodes' }).waitFor() } },
  { id: 'S32-pubsub', scenario: 'team', open: async (p, f) => {
    const t = await redisTab(p, 'cache-prod', 'Pub/Sub')
    for (const name of ['orders.*', 'deploys']) { await t.getByLabel('Channel or pattern').fill(name); await t.getByRole('button', { name: 'Listen', exact: true }).click() }
    await f.redisPublish('deploys', 'api v2.41.0 started')
    await f.redisPublish('orders.paid', '{"order":5512,"total":84.00}')
    await t.getByRole('table', { name: 'Messages' }).getByRole('row').nth(2).waitFor()
  } },
  { id: 'S33-confirm', scenario: 'team', open: async (p) => {
    const t = await redisTab(p, 'cache-prod'); await redisKey(t, 'profile')
    await t.getByRole('button', { name: 'Edit field plan' }).click()
    await p.getByTestId('redis-edit-row').getByLabel('Value').fill('business')
    await p.getByTestId('redis-edit-row').getByRole('button', { name: 'Review change' }).click()
    await p.getByTestId('redis-confirm').waitFor()
  } },
  { id: 'S33-danger', scenario: 'team', open: async (p) => {
    const t = await redisTab(p, 'cache-prod', 'Console')
    await t.getByRole('textbox', { name: 'Command' }).fill('FLUSHDB'); await p.keyboard.press('Enter')
    await p.getByTestId('redis-danger').getByLabel(/^Type cache-prod/).fill('cache-pr')
  } },
  { id: 'S33-bulk', scenario: 'team', open: async (p) => {
    const t = await redisTab(p, 'cache-prod'); await t.getByRole('button', { name: 'Bulk…' }).click()
    const d = p.getByTestId('redis-bulk')
    await d.getByLabel('Key pattern').fill('session:*'); await d.getByRole('button', { name: 'Preview' }).click()
    await d.getByTestId('bulk-preview').waitFor()
  } },
  { id: 'S33-write-mode', scenario: 'team', open: async (p) => { const t = await redisTab(p, 'sessions-prod'); await t.getByTestId('redis-mode').click(); await p.getByTestId('redis-mode-dialog').getByRole('radio', { name: /Confirm writes/ }).click() } },
  { id: 'S33-connecting', scenario: 'team', open: async (p, f) => {
    await f.redisManual()
    await vaultNav(p, 'Databases'); await p.getByRole('row', { name: /cache-prod/ }).dblclick()
    const conn = (await f.redisConnIds()).at(-1)!
    await f.redisEvent(conn, { type: 'plan', steps: [{ label: 'Sign in to bastion', detail: null }, { label: 'Sign in to db-1', detail: 'through bastion' }, { label: 'Reach Redis at 127.0.0.1:6379', detail: null }, { label: 'Sign in as app-ro', detail: null }] })
    await f.redisEvent(conn, { type: 'step', index: 0, state: 'done', detail: null })
    await f.redisEvent(conn, { type: 'step', index: 1, state: 'active', detail: null })
  } },
  { id: 'S23-settings-databases', scenario: 'team', open: async (p) => { await p.keyboard.press('ControlOrMeta+,'); await p.getByRole('button', { name: 'Databases', exact: true }).first().click() } },
  { id: 'S25-import', scenario: 'empty', open: async (p) => { await p.getByRole('button', { name: 'Import', exact: true }).click(); await p.getByRole('button', { name: 'Choose file…' }).click() } },
]
