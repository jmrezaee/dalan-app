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

/** Adds a MongoDB database and opens it in a tab. */
const mongoTab = async (page: Page) => {
  await addDb(page, 'MongoDB', 'docs-prod', 'mongo.internal:27017')
  await page.getByRole('row', { name: /docs-prod/ }).dblclick()
  const tab = page.getByTestId('mongo-tab-docs-prod')
  await tab.getByRole('tree', { name: 'Databases and collections' }).waitFor()
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
  { id: 'S44-mongo-tree', scenario: 'team', open: async (p) => { await mongoTab(p) } },
  { id: 'S44-mongo-docs', scenario: 'team', open: async (p) => { const t = await mongoTab(p); await t.getByTestId('mongo-coll-customers').click(); await t.getByTestId('mongo-doc').first().waitFor() } },
  { id: 'S44-mongo-table', scenario: 'team', open: async (p) => { const t = await mongoTab(p); await t.getByTestId('mongo-coll-customers').click(); await t.getByRole('group', { name: 'View' }).getByRole('button', { name: 'Table' }).click(); await t.getByRole('table', { name: 'Documents as a table' }).waitFor() } },
  { id: 'S45-mongo-aggregate', scenario: 'team', open: async (p) => { const t = await mongoTab(p); await t.getByTestId('mongo-coll-orders').click(); await t.getByTestId('mongo-section-aggregate').click(); await t.getByTestId('mongo-agg-run').click(); await t.getByTestId('mongo-agg-summary').waitFor() } },
  { id: 'S46-mongo-schema', scenario: 'team', open: async (p) => { const t = await mongoTab(p); await t.getByTestId('mongo-coll-orders').click(); await t.getByTestId('mongo-section-schema').click(); await t.getByTestId('mongo-field-status').waitFor() } },
  { id: 'S47-mongo-shell', scenario: 'team', open: async (p) => { const t = await mongoTab(p); await t.getByTestId('mongo-open-shell').click(); const i = t.getByTestId('mongo-shell-input'); await i.fill('use shop'); await i.press('Enter'); await i.fill('db.customers.find({})'); await i.press('Enter'); await t.getByTestId('mongo-shell-entry').first().waitFor() } },
  { id: 'S47-mongo-shell-danger', scenario: 'team', open: async (p) => { const t = await mongoTab(p); await t.getByTestId('mongo-open-shell').click(); const i = t.getByTestId('mongo-shell-input'); await i.fill('use shop'); await i.press('Enter'); await i.fill('db.customers.deleteMany({})'); await i.press('Enter'); await p.getByTestId('mongo-shell-review').waitFor() } },
  { id: 'S43-mongo-transfer', scenario: 'team', open: async (p) => { const t = await mongoTab(p); await t.getByTestId('mongo-coll-customers').click(); await t.getByTestId('mongo-open-transfer').click(); await t.getByTestId('mongo-transfer').locator('[data-part="tool"]').getByText(/mongodump/).waitFor() } },
  { id: 'S44-mongo-editor', scenario: 'team', open: async (p) => { const t = await mongoTab(p); await t.getByTestId('mongo-coll-customers').click(); await t.getByRole('button', { name: 'Edit document 1', exact: true }).click(); await p.getByTestId('mongo-editor-dialog').waitFor() } },
  { id: 'S44-mongo-review', scenario: 'team', open: async (p) => { const t = await mongoTab(p); await t.getByTestId('mongo-coll-customers').click(); await t.getByRole('button', { name: 'Delete document 2', exact: true }).click(); await p.getByTestId('mongo-review').waitFor() } },
  { id: 'S36-sql-tab-tree', scenario: 'team', open: async (p) => { await sqlTab(p) } },
  { id: 'S36-sql-table', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByRole('treeitem', { name: 'table orders' }).click(); await t.getByRole('grid', { name: 'public.orders rows' }).getByRole('gridcell', { name: '1' }).first().click() } },
  { id: 'S37-sql-editor', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByRole('button', { name: 'New query' }).click(); const box = t.getByRole('textbox', { name: /SQL$/ }); await box.click(); await p.keyboard.insertText('select * from customers'); await t.getByTestId('sql-run').click(); await t.getByRole('button', { name: /Result 1/ }).waitFor() } },
  { id: 'S42-sql-confirm', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByRole('button', { name: 'New query' }).click(); await t.getByRole('textbox', { name: /SQL$/ }).click(); await p.keyboard.insertText('update customers set name = \'x\' where id = 1'); await t.getByTestId('sql-run').click(); await p.getByTestId('sql-confirm').waitFor() } },
  { id: 'S42-sql-danger', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByRole('button', { name: 'New query' }).click(); await t.getByRole('textbox', { name: /SQL$/ }).click(); await p.keyboard.insertText('delete from orders'); await t.getByTestId('sql-run').click(); await p.getByTestId('sql-danger').waitFor() } },
  { id: 'S42-sql-commit', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByTestId('sql-object-customers').click(); const g = t.getByRole('grid', { name: 'public.customers rows' }); await g.getByRole('row').nth(1).getByRole('gridcell').nth(1).dblclick(); await p.keyboard.insertText('Ada'); await p.keyboard.press('Enter'); await t.getByRole('button', { name: 'Review changes' }).click(); await p.getByTestId('sql-commit').waitFor() } },
  { id: 'S38-sql-structure', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByRole('treeitem', { name: 'table orders' }).click(); await t.getByRole('group', { name: 'View' }).getByRole('button', { name: 'Structure' }).click(); await t.getByTestId('structure-col-id').waitFor() } },
  { id: 'S39-sql-explain', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByRole('button', { name: 'New query' }).click(); await t.getByRole('textbox', { name: /SQL$/ }).click(); await p.keyboard.insertText('select * from orders where status = \'paid\''); await t.getByTestId('sql-explain-analyze').click(); await t.getByTestId('plan-hint').waitFor() } },
  { id: 'S40-sql-diagram', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByRole('treeitem', { name: 'table orders' }).click(); await t.getByTestId('sql-new-diagram').click(); await t.getByTestId('diagram-box-orders').waitFor() } },
  { id: 'S41-sql-activity', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByTestId('sql-open-activity').click(); await t.getByTestId('session-18342').waitFor() } },
  { id: 'S41-session-confirm', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByTestId('sql-open-activity').click(); await t.getByRole('button', { name: 'End session 18290' }).click(); await p.getByTestId('session-confirm').waitFor() } },
  { id: 'S43-sql-transfer', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByTestId('sql-open-transfer').click(); await t.getByTestId('sql-transfer').getByRole('button', { name: 'Choose file…' }).click(); await t.getByRole('table', { name: 'Column mapping' }).waitFor() } },
  // D13: S49 "Copy data": choosing (this tab as its own target), the review, the report.
  { id: 'S49-copy-choose', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByTestId('sql-open-copy').click(); await t.getByRole('checkbox', { name: 'Copy public.customers' }).check() } },
  { id: 'S49-copy-review', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByTestId('sql-open-copy').click(); await t.getByRole('checkbox', { name: 'Copy public.customers' }).check(); await t.getByRole('textbox', { name: 'Name at the target for customers' }).fill('customers2'); await t.getByTestId('copy-review').click(); await t.getByTestId('copy-plan').waitFor() } },
  { id: 'S49-copy-report', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByTestId('sql-open-copy').click(); await t.getByRole('checkbox', { name: 'Copy public.no_key' }).check(); await t.getByRole('checkbox', { name: 'Into a new database on that server' }).check(); await t.getByRole('textbox', { name: 'New database name' }).fill('scratch'); await t.getByTestId('copy-review').click(); await t.getByTestId('copy-run').click(); await t.getByTestId('copy-report').waitFor() } },
  { id: 'S49-copy-mapping', scenario: 'team', open: async (p) => { await addDb(p, 'MySQL', 'my-target', 'my.internal'); await p.getByRole('row', { name: /my-target/ }).dblclick(); await p.getByTestId('sql-tab-my-target').getByRole('tree', { name: 'Schemas and tables' }).waitFor(); await p.getByRole('tab', { name: 'Vault' }).click(); const t = await sqlTab(p); await t.getByTestId('sql-open-copy').click(); await t.getByRole('combobox', { name: 'Target database' }).selectOption({ label: 'my-target · MySQL' }); await t.getByRole('checkbox', { name: 'Copy public.customers' }).check(); await t.getByTestId('copy-review').click(); await t.getByTestId('copy-mapping').locator('summary').click(); await t.getByRole('textbox', { name: 'Target type for created' }).waitFor() } },
  // D12: S48 "Restore a dump", one entry per step that has its own content.
  { id: 'S48-restore-inside', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByTestId('sql-open-transfer').click(); await t.getByTestId('sql-restore').click(); await p.getByTestId('restore-pick-file').click(); await p.getByRole('table', { name: 'Objects in the dump' }).waitFor() } },
  { id: 'S48-restore-target', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByTestId('sql-open-transfer').click(); await t.getByTestId('sql-restore').click(); await p.getByTestId('restore-pick-file').click(); await p.getByRole('button', { name: 'Next' }).click(); await p.getByRole('radio', { name: 'Schema only' }).waitFor() } },
  { id: 'S48-restore-review', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByTestId('sql-open-transfer').click(); await t.getByTestId('sql-restore').click(); await p.getByTestId('restore-pick-file').click(); await p.getByRole('button', { name: 'Next' }).click(); await p.getByTestId('restore-next').click(); await p.getByTestId('restore-typed').waitFor() } },
  { id: 'S48-restore-refused', scenario: 'team', open: async (p, fake) => { await fake.restoreNext('hostile'); const t = await sqlTab(p); await t.getByTestId('sql-open-transfer').click(); await t.getByTestId('sql-restore').click(); await p.getByTestId('restore-pick-file').click(); await p.locator('[data-part=refused]').waitFor() } },
  { id: 'S48-restore-result', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByTestId('sql-open-transfer').click(); await t.getByTestId('sql-restore').click(); await p.getByTestId('restore-pick-file').click(); await p.getByRole('button', { name: 'Next' }).click(); await p.getByRole('radio', { name: 'A new database on the same server' }).check(); await p.getByTestId('restore-new-name').fill('copy'); await p.getByTestId('restore-next').click(); await p.getByTestId('restore-confirm').click(); await p.getByTestId('restore-result').waitFor() } },
  { id: 'S34-saved-queries', scenario: 'team', open: async (p) => { await vaultNav(p, 'Saved queries') } },
  { id: 'E2-schedule', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByTestId('sql-open-transfer').click(); const b = t.getByTestId('sql-transfer').getByRole('region', { name: 'Backup' }); await b.getByRole('button', { name: 'Save as a job…' }).click(); await b.getByRole('textbox', { name: 'Job name' }).fill('Nightly'); await b.getByTestId('backup-save-job').click(); await p.getByRole('tab', { name: 'Vault' }).click(); await vaultNav(p, 'Jobs'); await p.getByTestId('job-Nightly').getByTestId('job-schedule').click(); await p.getByTestId('schedule-dialog').getByRole('radio', { name: 'Every day' }).check(); await p.getByTestId('schedule-dialog').locator('[data-part=preview]').waitFor() } },
  { id: 'E8-host-picker', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByTestId('sql-open-transfer').click(); await t.getByTestId('sql-restore').click(); await p.getByTestId('restore-pick-host').click(); const k = p.getByTestId('host-picker'); await k.getByRole('combobox', { name: 'Host' }).selectOption({ label: 'bastion' }); await k.getByRole('button', { name: 'Connect' }).click(); await k.getByRole('textbox', { name: 'Folder on the host' }).waitFor() } },
  { id: 'E3-schema-compare', scenario: 'team', open: async (p) => { await addDb(p, 'PostgreSQL', 'stage-db', 'stage.internal'); await p.getByRole('row', { name: /stage-db/ }).dblclick(); await p.getByTestId('sql-tab-stage-db').getByRole('tree', { name: 'Schemas and tables' }).waitFor(); await p.getByRole('tab', { name: 'Vault' }).click(); const t = await sqlTab(p); await t.getByTestId('sql-open-compare').click(); await t.getByTestId('compare-run').click(); await t.getByTestId('compare-sync').click(); await t.getByTestId('compare-plan').waitFor() } },
  { id: 'E4-data-compare', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByTestId('sql-open-compare').click(); const c = t.getByTestId('compare-tab'); await c.getByRole('radiogroup', { name: 'Compare' }).getByText('Data in a table').click(); await c.getByRole('textbox', { name: 'Source table: name' }).fill('customers'); await c.getByTestId('data-columns').click(); await c.getByTestId('data-compare').click(); await c.getByTestId('data-match').click(); await c.getByTestId('data-match-plan').waitFor() } },
  { id: 'E1-jobs-empty', scenario: 'team', open: async (p) => { await vaultNav(p, 'Jobs'); await p.getByText('No saved jobs yet').waitFor() } },
  { id: 'E1-jobs-history', scenario: 'team', open: async (p) => { const t = await sqlTab(p); await t.getByTestId('sql-open-transfer').click(); const b = t.getByTestId('sql-transfer').getByRole('region', { name: 'Backup' }); await b.getByRole('button', { name: 'Save as a job…' }).click(); await b.getByRole('textbox', { name: 'Job name' }).fill('Nightly'); await b.getByTestId('backup-save-job').click(); await p.getByRole('tab', { name: 'Vault' }).click(); await vaultNav(p, 'Jobs'); await p.getByTestId('job-Nightly').getByRole('button', { name: 'History' }).click(); await p.getByText('No runs yet.').waitFor() } },
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
