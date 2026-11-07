import { expect } from '@playwright/test'
import { chapter, MASTER } from './chapter'
import { click, dblclick, fill, press, step } from './step'
import { unlock } from '../e2e/harness'

// Chapter 14 (S15, S20): session logs with a bookmark and a note, a replay, and the History tab turning a repeated
// command into a snippet.
chapter(14, { scenario: 'team' }, async (page, fake) => {
  await unlock(page, MASTER)

  await step(page, { id: '14-01', screen: 'S15', caption: 'Logs: each server keeps its latest session; one is bookmarked' }, async () => {
    await click(page, page.getByLabel('Vault', { exact: true }).getByText('Logs'))
    await expect(page.getByLabel('Session logs')).toBeVisible()
    await expect(page.getByText('The nginx reload that fixed the 502s').or(page.getByRole('textbox', { name: /Note for the web-1 log/ }))).toBeVisible()
  })

  await step(page, { id: '14-02', screen: 'S15', caption: 'Bookmark another log and give it a note' }, async () => {
    await click(page, page.getByRole('button', { name: 'Bookmark', exact: true }).first())
    await fill(page, page.getByRole('textbox', { name: /Note for the db-1 log/ }), 'Index rebuild, 40 minutes')
    await press(page, 'Enter')
    await expect(page.getByRole('button', { name: 'Remove bookmark' })).toHaveCount(2)
  })

  await step(page, { id: '14-03', screen: 'S15', caption: 'Replay a session in a terminal' }, async () => {
    await click(page, page.getByRole('button', { name: 'Replay' }).first())
    await expect(page.getByRole('dialog').getByLabel('Log replay')).toBeVisible()
    await expect.poll(async () => (await fake.calls('log_replay')).length).toBe(1)
    await expect(page.getByRole('dialog').locator('.xterm')).toBeVisible()
    await press(page, 'Escape')
  })

  await step(page, { id: '14-04', screen: 'S20', caption: 'History: a command you keep typing becomes a snippet' }, async () => {
    await click(page, page.getByLabel('Vault', { exact: true }).getByText('Hosts', { exact: true }))
    await click(page, page.getByText('Production', { exact: true }).first())
    await dblclick(page, page.getByRole('row').filter({ has: page.getByText('web-1', { exact: true }) }).first())
    const id = (await fake.sessionIds())[0]!
    await fake.session(id).state('ready')
    await click(page, page.getByRole('button', { name: /^Snippets/ }))
    await click(page, page.getByRole('tab', { name: 'History', exact: true }))
    await expect(page.getByText('tail -f /var/log/nginx/error.log').first()).toBeVisible()
    await expect(page.getByText(/Save it as a snippet\?/)).toBeVisible()
    await click(page, page.getByRole('button', { name: 'Save snippet' }))
    await expect(page.getByText(/^Saved .+\./)).toBeVisible()
  })
})
