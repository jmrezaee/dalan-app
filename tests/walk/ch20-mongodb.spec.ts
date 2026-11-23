import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, dblclick, fill, press, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 20 (replica/databases/spec.md; S44 to S47, F27 to F30): a MongoDB database from the list to a tab, a
// collection's documents, an edit that is shown first, an aggregation, the schema and indexes, and the shell with its gate.
chapter(20, { scenario: 'team' }, async (page) => {
  await unlock(page, MASTER)
  const nav = page.getByRole('navigation', { name: 'Vault' })
  const tab = page.getByTestId('mongo-tab-docs-prod')

  await step(page, { id: '20-01', flow: 'F27', screen: 'S44', caption: 'Open a MongoDB database: databases and collections on the left' }, async () => {
    await click(page, nav.getByRole('link', { name: /Databases/ }))
    await click(page, page.getByRole('button', { name: 'New database' }))
    await click(page, page.getByTestId('new-database-dialog').getByRole('radio', { name: /MongoDB/ }))
    await click(page, page.getByTestId('new-database-dialog').getByRole('button', { name: 'Continue' }))
    const s = page.getByTestId('db-form-sheet')
    await fill(page, s.getByLabel('Name', { exact: true }), 'docs-prod')
    await fill(page, s.getByLabel('Members'), 'mongo.internal:27017')
    await click(page, s.getByRole('button', { name: 'Save' }))
    await dblclick(page, page.getByRole('row', { name: /docs-prod/ }))
    await expect(tab.getByRole('tree', { name: 'Databases and collections' })).toBeVisible()
  })

  await step(page, { id: '20-02', flow: 'F27', screen: 'S44', caption: 'A collection: filter it, switch between list, table and JSON' }, async () => {
    await click(page, tab.getByTestId('mongo-coll-customers'))
    await fill(page, tab.getByTestId('mongo-filter'), '{"vip": true}')
    await click(page, tab.getByTestId('mongo-apply'))
    await expect(tab.getByTestId('mongo-doc')).toHaveCount(4)
    await click(page, tab.getByRole('group', { name: 'View' }).getByRole('button', { name: 'Table' }))
    await expect(tab.getByRole('table', { name: 'Documents as a table' })).toBeVisible()
  })

  await step(page, { id: '20-03', flow: 'F27', screen: 'S44', caption: 'Delete a document: Dalan says what it will do and asks first' }, async () => {
    await click(page, tab.getByRole('textbox', { name: 'Filter' }).first())
    await fill(page, tab.getByTestId('mongo-filter'), '')
    await click(page, tab.getByTestId('mongo-apply'))
    await click(page, tab.getByRole('button', { name: 'Delete document 2', exact: true }))
    await expect(page.getByTestId('mongo-review')).toContainText('This can\'t be undone')
    await click(page, page.getByTestId('mongo-review').getByRole('button', { name: 'Cancel' }))
  })

  await step(page, { id: '20-04', flow: 'F28', screen: 'S45', caption: 'An aggregation: stages that switch off, and the output after each' }, async () => {
    await click(page, tab.getByTestId('mongo-section-aggregate'))
    await fill(page, tab.getByRole('textbox', { name: 'Stage 1 $match body' }), '{"vip": true}')
    await click(page, tab.getByTestId('mongo-agg-run'))
    await expect(tab.getByTestId('mongo-agg-summary')).toContainText('4 documents')
  })

  await step(page, { id: '20-05', flow: 'F29', screen: 'S46', caption: 'The schema from a sample, and the indexes with how often they are used' }, async () => {
    await click(page, tab.getByTestId('mongo-section-schema'))
    await expect(tab.getByTestId('mongo-field-name')).toContainText('string')
    await expect(tab.getByTestId('mongo-index-status_1')).toContainText('40')
  })

  await step(page, { id: '20-06', flow: 'F30', screen: 'S47', caption: 'The shell: familiar commands, no JavaScript, and a dangerous one asks for the name' }, async () => {
    await click(page, tab.getByTestId('mongo-open-shell'))
    const input = tab.getByTestId('mongo-shell-input')
    await fill(page, input, 'use shop')
    await press(page, 'Enter')
    await fill(page, input, 'db.customers.deleteMany({})')
    await press(page, 'Enter')
    await expect(page.getByTestId('mongo-shell-review')).toContainText('EVERY document')
    await click(page, page.getByRole('button', { name: 'Cancel' }))
  })
})
