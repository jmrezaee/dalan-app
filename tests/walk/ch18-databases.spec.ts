import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, dblclick, fill, press, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 18 (replica/redis/spec.md; S26 to S33, F13 to F19): a Redis database from the list to a tab, its keys
// and values, a change that is reviewed first, the console, listening to a channel, a bulk action, the server
// and a cluster's nodes, and a production database that stays Read-only until the master password is typed.
chapter(18, { scenario: 'team' }, async (page, fake) => {
  await unlock(page, MASTER)
  const nav = page.getByRole('navigation', { name: 'Vault' })
  const row = (name: string) => page.getByRole('row', { name: new RegExp(name) })
  const tabOf = (name: string) => page.getByTestId(`redis-tab-${name}`)
  const confirm = page.getByTestId('redis-confirm')

  await step(page, { id: '18-01', flow: 'F13', screen: 'S26', caption: 'Databases lists what is saved, with how each is reached and what may be written' }, async () => {
    await click(page, nav.getByRole('link', { name: /Databases/ }))
    await expect(row('cache-prod')).toContainText('production')
    await expect(row('cache-prod')).toContainText('Confirm writes')
    await expect(row('sessions-prod')).toContainText('Read-only')
  })

  await step(page, { id: '18-02', flow: 'F13', screen: 'S27', caption: 'A new database: tested, then saved; the password never comes back' }, async () => {
    await click(page, page.getByRole('button', { name: 'New database' }))
    await click(page, page.getByRole('radio', { name: /Redis/ }))
    await click(page, page.getByRole('button', { name: 'Continue' }))
    const s = page.getByRole('dialog', { name: /database/i })
    await fill(page, s.getByLabel('Name', { exact: true }), 'jobs-staging')
    await fill(page, s.getByLabel('Address'), 'localhost')
    await click(page, s.getByRole('button', { name: 'Test connection' }))
    await expect(s.getByRole('status')).toContainText('Connected · Redis 7.2')
    await click(page, s.getByRole('button', { name: 'Save' }))
    await expect(row('jobs-staging')).toBeVisible()
  })

  const tab = tabOf('cache-prod')
  await step(page, { id: '18-03', flow: 'F13', screen: 'S33', caption: 'Opening a production database: a tab with an amber line, connected through its host' }, async () => {
    await dblclick(page, row('cache-prod'))
    await expect(tab.getByRole('tree', { name: 'Keys' })).toBeVisible()
    await expect(tab.getByRole('heading', { name: 'cache-prod' })).toBeVisible()
    await expect(tab.getByTestId('redis-mode')).toHaveText('Confirm writes')
  })

  await step(page, { id: '18-04', flow: 'F14', screen: 'S28', caption: 'Find a key in the tree and read its fields' }, async () => {
    await click(page, tab.getByRole('treeitem', { name: /^user:/ }))
    await click(page, tab.getByRole('treeitem', { name: /^1042:/ }))
    await click(page, tab.getByRole('treeitem', { name: /profile/ }).first())
    await expect(tab.getByRole('table', { name: 'Fields' }).getByRole('row', { name: /plan/ })).toContainText('team')
  })

  await step(page, { id: '18-05', flow: 'F15', screen: 'S33', caption: 'Change a field: Dalan shows what it will send, and Cancel is the focused button' }, async () => {
    await click(page, tab.getByRole('button', { name: 'Edit field plan' }))
    await fill(page, page.getByTestId('redis-edit-row').getByLabel('Value'), 'business')
    await click(page, page.getByTestId('redis-edit-row').getByRole('button', { name: 'Review change' }))
    await expect(confirm.locator('[data-part="command"]')).toHaveText('HSET user:1042:profile plan business')
    await expect(confirm.getByRole('button', { name: 'Cancel' })).toBeFocused()
  })

  await step(page, { id: '18-06', flow: 'F15', screen: 'S29', caption: 'Confirm it: the field changes' }, async () => {
    await click(page, confirm.getByRole('button', { name: 'Change', exact: true }))
    await expect(tab.getByRole('table', { name: 'Fields' }).getByRole('row', { name: /plan/ })).toContainText('business')
  })

  await step(page, { id: '18-07', flow: 'F16', screen: 'S30', caption: 'The console: a read runs, KEYS is held back with a safer command on offer' }, async () => {
    await click(page, tab.getByRole('tab', { name: 'Console' }))
    const input = tab.getByRole('textbox', { name: 'Command' })
    await fill(page, input, 'TTL user:1042:profile')
    await press(page, 'Enter')
    await expect(tab.getByTestId('console-entry').last()).toContainText('(integer) 83640')
    await fill(page, input, 'KEYS *')
    await press(page, 'Enter')
    await expect(tab.getByTestId('console-entry').last()).toContainText('Not sent. KEYS walks every key at once')
  })

  await step(page, { id: '18-08', flow: 'F17', screen: 'S32', caption: 'Listen to a pattern: messages arrive while the rest of the tab keeps working' }, async () => {
    await click(page, tab.getByRole('tab', { name: 'Pub/Sub' }))
    await fill(page, tab.getByLabel('Channel or pattern'), 'orders.*')
    await click(page, tab.getByRole('button', { name: 'Listen', exact: true }))
    await fake.redisPublish('orders.paid', '{"order":5512,"total":84.00}')
    await expect(tab.getByRole('table', { name: 'Messages' }).getByRole('row').nth(1)).toContainText('orders.paid')
  })

  await step(page, { id: '18-09', flow: 'F18', screen: 'S33', caption: 'A bulk action: preview what matches, then type the database\'s name' }, async () => {
    await click(page, tab.getByRole('tab', { name: 'Browser' }))
    await click(page, tab.getByRole('button', { name: 'Bulk…' }))
    const d = page.getByTestId('redis-bulk')
    await fill(page, d.getByLabel('Key pattern'), 'session:*')
    await click(page, d.getByRole('button', { name: 'Preview' }))
    await expect(d.getByTestId('bulk-preview')).toContainText('1,200 keys match')
    await expect(d.getByRole('button', { name: 'Delete 1,200 keys' })).toBeDisabled()
    await fill(page, d.getByLabel('Type cache-prod to confirm'), 'cache-prod')
    await click(page, d.getByRole('button', { name: 'Delete 1,200 keys' }))
    await expect(d.getByTestId('bulk-result')).toContainText('Deleted 1,200 keys.')
    await click(page, d.locator('[data-cancel]'))
  })

  await step(page, { id: '18-10', screen: 'S31', caption: 'The Server view: what the server says about itself, and its slow commands' }, async () => {
    await click(page, tab.getByRole('tab', { name: 'Server' }))
    await expect(tab.getByRole('table', { name: 'Slow commands' })).toBeVisible()
    await expect(tab.getByTestId('redis-server')).toContainText('7.2.4')
  })

  await step(page, { id: '18-11', flow: 'F19', screen: 'S31', caption: 'A cluster opens like any other database and lists its nodes' }, async () => {
    await click(page, page.getByRole('tab', { name: 'Vault' }))
    await dblclick(page, row('events-cluster'))
    const cluster = tabOf('events-cluster')
    await expect(cluster.getByRole('tree', { name: 'Keys' })).toBeVisible()
    await click(page, cluster.getByRole('tab', { name: 'Server' }))
    await expect(cluster.getByRole('table', { name: 'Nodes' }).getByRole('row').nth(1)).toContainText('0–5460')
  })

  await step(page, { id: '18-12', screen: 'S33', caption: 'A Read-only database: nothing to edit, and the console refuses a write before it is sent' }, async () => {
    await click(page, page.getByRole('tab', { name: 'Vault' }))
    await dblclick(page, row('sessions-prod'))
    const ro = tabOf('sessions-prod')
    await expect(ro.getByTestId('redis-read-only')).toContainText('Read-only. Nothing here can change data.')
    await expect(ro.getByRole('button', { name: 'New key' })).toHaveCount(0)
    await click(page, ro.getByRole('tab', { name: 'Console' }))
    await fill(page, ro.getByRole('textbox', { name: 'Command' }), 'DEL maintenance')
    await press(page, 'Enter')
    await expect(ro.getByTestId('console-entry').last()).toContainText('Not sent. sessions-prod is read-only in Dalan.')
  })

  await step(page, { id: '18-13', screen: 'S33', caption: 'Loosening a production database asks for the master password' }, async () => {
    const ro = tabOf('sessions-prod')
    await click(page, ro.getByTestId('redis-read-only').getByRole('button', { name: 'Change…' }))
    const d = page.getByTestId('redis-mode-dialog')
    await click(page, d.getByRole('radio', { name: /Confirm writes/ }))
    await fill(page, d.getByLabel('Master password'), MASTER)
    await click(page, d.getByRole('button', { name: 'Change mode' }))
    await expect(ro.getByTestId('redis-read-only')).toHaveCount(0)
    await expect(ro.getByTestId('redis-mode')).toHaveText('Confirm writes')
  })
})
