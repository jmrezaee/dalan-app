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
  { id: 'S25-import', scenario: 'empty', open: async (p) => { await p.getByRole('button', { name: 'Import', exact: true }).click(); await p.getByRole('button', { name: 'Choose file…' }).click() } },
]
