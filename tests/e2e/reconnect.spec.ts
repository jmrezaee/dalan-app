import { expect, test } from '../support/fixtures'
import { openApp, seen } from './harness'

// F11: automatic reconnect on the page clock. The schedule is 2, 4, 8, 16 and 30 seconds, then it stops.
test('F11: with automatic reconnect on, retries come after 2, 4, 8, 16 and 30 seconds, then stop', async ({ page }) => {
  const fake = await openApp(page, { scenario: 'team', unlocked: true })
  await expect(page.getByLabel('Vault', { exact: true })).toBeVisible()
  await page.keyboard.press('ControlOrMeta+,')
  await page.getByRole('button', { name: 'Terminal', exact: true }).or(page.getByRole('tab', { name: 'Terminal', exact: true })).first().click()
  await page.getByRole('switch', { name: 'Reconnect automatically' }).click()
  await page.getByRole('tab', { name: 'Vault' }).click()
  await page.getByLabel('Vault', { exact: true }).getByText('Hosts', { exact: true }).click()
  await page.getByRole('row').filter({ has: page.getByText('bastion', { exact: true }) }).first().dblclick()
  const [id] = await fake.sessionIds()
  await fake.session(id!).state('ready')
  const tries = async () => (await fake.calls()).filter(c => c.startsWith('reconnect:')).length
  // Freeze time here, so only `runFor` moves it and the waits below are exact.
  await page.clock.pauseAt(await page.evaluate(() => Date.now() + 1000))
  await fake.session(id!).state('disconnected')
  await page.clock.runFor(1900)
  expect(await tries()).toBe(0)
  await page.clock.runFor(200)
  await expect.poll(tries).toBe(1) // 2 s
  for (const [i, wait] of [4, 8, 16, 30].entries()) {
    await fake.session(id!).error('host_unreachable', 'down') // the server is still down
    await page.clock.runFor(wait * 1000 - 100)
    expect(await tries(), `before the ${wait} s retry`).toBe(i + 1)
    await page.clock.runFor(200)
    await expect.poll(tries).toBe(i + 2)
  }
  await fake.session(id!).error('host_unreachable', 'down')
  await seen(page.getByText(/Couldn't reconnect to bastion/)) // it says so when it gives up
  await page.clock.runFor(120_000)
  expect(await tries()).toBe(5) // and does not try again
})

test('F11: a wrong password never triggers an automatic retry', async ({ page }) => {
  const fake = await openApp(page, { scenario: 'team', unlocked: true })
  await expect(page.getByLabel('Vault', { exact: true })).toBeVisible()
  await page.keyboard.press('ControlOrMeta+,')
  await page.getByRole('button', { name: 'Terminal', exact: true }).or(page.getByRole('tab', { name: 'Terminal', exact: true })).first().click()
  await page.getByRole('switch', { name: 'Reconnect automatically' }).click()
  await page.getByRole('tab', { name: 'Vault' }).click()
  await page.getByLabel('Vault', { exact: true }).getByText('Hosts', { exact: true }).click()
  await page.getByRole('row').filter({ has: page.getByText('bastion', { exact: true }) }).first().dblclick()
  const [id] = await fake.sessionIds()
  await fake.session(id!).state('ready')
  await fake.session(id!).state('disconnected')
  await page.clock.runFor(2100)
  await expect.poll(async () => (await fake.calls()).filter(c => c.startsWith('reconnect:')).length).toBe(1)
  await fake.session(id!).error('auth_failed', 'wrong password')
  await page.clock.runFor(120_000)
  expect((await fake.calls()).filter(c => c.startsWith('reconnect:')).length).toBe(1)
})
