import { expect, test } from '../support/fixtures'
import { openApp } from './harness'

// D11 of SQL and MongoDB: a page of 500 documents draws and switches views quickly. Timing test: `pnpm test:perf`.
const median = (xs: number[]) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)]!

test('a MongoDB page of 500 documents draws in the list and the table within a second @perf', async ({ page }) => {
  await openApp(page, { scenario: 'team', unlocked: true })
  await page.getByRole('navigation', { name: 'Vault' }).getByRole('link', { name: /Databases/ }).click()
  await page.getByRole('button', { name: 'New database' }).click()
  await page.getByTestId('new-database-dialog').getByRole('radio', { name: /MongoDB/ }).click()
  await page.getByTestId('new-database-dialog').getByRole('button', { name: 'Continue' }).click()
  const s = page.getByTestId('db-form-sheet')
  await s.getByLabel('Name', { exact: true }).fill('docs-perf')
  await s.getByLabel('Members').fill('mongo.internal:27017')
  await s.getByRole('button', { name: 'Save' }).click()
  await page.getByRole('row', { name: /docs-perf/ }).dblclick()
  const tab = page.getByTestId('mongo-tab-docs-perf')
  await tab.getByTestId('mongo-coll-orders').click()
  await expect(tab.getByTestId('mongo-doc')).toHaveCount(50)
  const times: number[] = []
  for (let i = 0; i < 3; i++) {
    const t0 = Date.now()
    await tab.getByLabel('Documents per page').selectOption(i % 2 ? '25' : '500')
    await expect(tab.getByTestId('mongo-doc')).toHaveCount(i % 2 ? 25 : 500)
    times.push(Date.now() - t0)
  }
  console.log(`500 documents in the list: median ${median(times)} ms`)
  expect(median(times)).toBeLessThan(1000)
  await tab.getByLabel('Documents per page').selectOption('500')
  await expect(tab.getByTestId('mongo-doc')).toHaveCount(500)
  const t1 = Date.now()
  await tab.getByRole('group', { name: 'View' }).getByRole('button', { name: 'Table' }).click()
  await expect(tab.getByRole('table', { name: 'Documents as a table' }).getByRole('row')).toHaveCount(501)
  console.log(`500 documents as a table: ${Date.now() - t1} ms`)
  expect(Date.now() - t1).toBeLessThan(1000)
})
